//! Masking: sensitive values become typed placeholders before anything leaves,
//! and come back only inside the trust boundary.
//!
//! The mapping (the [`Vault`]) lives in memory for one call. Placeholders are
//! numbered in order of first appearance; a conversation is re-sent whole on each
//! turn and only grows at its end, so the same value gets the same placeholder on
//! every turn without anything being stored.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::sync::{Arc, OnceLock};

use regex::Regex;
use seiva_llm::{ChatRequest, ChatResponse, Content};
use serde_json::Value;

use crate::detect::Detector;

/// The value ↔ placeholder mapping of one call.
#[derive(Default, Clone)]
pub struct Vault {
    by_value: HashMap<String, String>,
    by_placeholder: HashMap<String, String>,
    counters: BTreeMap<String, usize>,
}

impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the values.
        f.debug_struct("Vault")
            .field("counts", &self.counters)
            .finish()
    }
}

impl Vault {
    fn placeholder(&mut self, label: &str, value: &str) -> String {
        if let Some(p) = self.by_value.get(value) {
            return p.clone();
        }
        let n = self.counters.entry(label.to_owned()).or_insert(0);
        *n += 1;
        let p = format!("‹{label}_{n}›");
        self.by_value.insert(value.to_owned(), p.clone());
        self.by_placeholder.insert(p.clone(), value.to_owned());
        p
    }

    /// How many distinct values were masked, per label.
    pub fn counts(&self) -> &BTreeMap<String, usize> {
        &self.counters
    }

    /// Whether nothing was masked.
    pub fn is_empty(&self) -> bool {
        self.by_value.is_empty()
    }

    /// `text` with this call's placeholders restored. A placeholder the vault did
    /// not create (one the caller typed) is left alone.
    pub fn unmask(&self, text: &str) -> String {
        if self.by_placeholder.is_empty() || !text.contains('‹') {
            return text.to_owned();
        }
        placeholder_regex()
            .replace_all(text, |c: &regex::Captures<'_>| {
                let p = &c[0];
                self.by_placeholder
                    .get(p)
                    .cloned()
                    .unwrap_or_else(|| p.to_owned())
            })
            .into_owned()
    }

    /// Every string inside `value`, unmasked.
    pub fn unmask_json(&self, value: &Value) -> Value {
        map_strings(value, &mut |s| self.unmask(s))
    }
}

fn placeholder_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"‹[A-Z][A-Z0-9_]*_\d+›").expect("valid pattern"))
}

/// The request carried a credential (which one is never said).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialFound;

/// Applies the detectors to requests and texts.
#[derive(Clone)]
pub struct Masker {
    detectors: Vec<Arc<dyn Detector>>,
}

impl Masker {
    /// A masker over `detectors`, in priority order.
    pub fn new(detectors: Vec<Arc<dyn Detector>>) -> Self {
        Self { detectors }
    }

    /// The non-overlapping findings in `text`: `(detector index, range)`, leftmost
    /// first, the longest at a position, the earlier detector on a tie.
    pub fn scan(&self, text: &str) -> Vec<(usize, Range<usize>)> {
        let mut all: Vec<(usize, Range<usize>)> = self
            .detectors
            .iter()
            .enumerate()
            .flat_map(|(i, d)| d.find(text).into_iter().map(move |r| (i, r)))
            .filter(|(_, r)| !r.is_empty())
            .collect();
        all.sort_by(|(ia, a), (ib, b)| {
            a.start
                .cmp(&b.start)
                .then(b.len().cmp(&a.len()))
                .then(ia.cmp(ib))
        });
        let mut out: Vec<(usize, Range<usize>)> = Vec::new();
        for (i, r) in all {
            if out.last().map_or(true, |(_, last)| r.start >= last.end) {
                out.push((i, r));
            }
        }
        out
    }

    /// Whether `text` carries a credential.
    pub fn has_credential(&self, text: &str) -> bool {
        self.scan(text)
            .iter()
            .any(|(i, _)| self.detectors[*i].is_credential())
    }

    /// Every label found in `text`, credentials included.
    pub fn labels_in(&self, text: &str) -> Vec<String> {
        self.scan(text)
            .iter()
            .map(|(i, _)| self.detectors[*i].label().to_owned())
            .collect()
    }

    /// `text` with every finding replaced by its placeholder. With
    /// `refuse_credentials`, a credential stops the masking.
    pub fn mask_text(
        &self,
        text: &str,
        vault: &mut Vault,
        refuse_credentials: bool,
    ) -> Result<String, CredentialFound> {
        let findings = self.scan(text);
        if findings.is_empty() {
            return Ok(text.to_owned());
        }
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        for (i, r) in findings {
            let d = &self.detectors[i];
            if d.is_credential() && refuse_credentials {
                return Err(CredentialFound);
            }
            out.push_str(&text[at..r.start]);
            out.push_str(&vault.placeholder(d.label(), &text[r.clone()]));
            at = r.end;
        }
        out.push_str(&text[at..]);
        Ok(out)
    }

    /// The request with every text masked: the system prompt, message text, tool
    /// results and the strings inside earlier tool calls. Images are not read.
    pub fn mask_request(
        &self,
        request: &ChatRequest,
        vault: &mut Vault,
        refuse_credentials: bool,
    ) -> Result<ChatRequest, CredentialFound> {
        let mut out = request.clone();
        if let Some(system) = &request.system {
            out.system = Some(self.mask_text(system, vault, refuse_credentials)?);
        }
        for message in &mut out.messages {
            for part in &mut message.content {
                match part {
                    Content::Text { text } => {
                        *text = self.mask_text(text, vault, refuse_credentials)?
                    }
                    Content::ToolResult { content, .. } => {
                        *content = self.mask_text(content, vault, refuse_credentials)?
                    }
                    Content::ToolUse { input, .. } => {
                        let mut failed = false;
                        let masked = map_strings(input, &mut |s| match self.mask_text(
                            s,
                            vault,
                            refuse_credentials,
                        ) {
                            Ok(m) => m,
                            Err(_) => {
                                failed = true;
                                String::new()
                            }
                        });
                        if failed {
                            return Err(CredentialFound);
                        }
                        *input = masked;
                    }
                    _ => {}
                }
            }
        }
        Ok(out)
    }
}

/// The response with this call's placeholders restored in text and tool inputs.
pub fn unmask_response(response: &ChatResponse, vault: &Vault) -> ChatResponse {
    let mut out = response.clone();
    for part in &mut out.content {
        match part {
            Content::Text { text } => *text = vault.unmask(text),
            Content::ToolUse { input, .. } => *input = vault.unmask_json(input),
            _ => {}
        }
    }
    out
}

/// Every string in `value` (object keys excluded) through `f`.
fn map_strings(value: &Value, f: &mut dyn FnMut(&str) -> String) -> Value {
    match value {
        Value::String(s) => Value::String(f(s)),
        Value::Array(items) => Value::Array(items.iter().map(|v| map_strings(v, f)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), map_strings(v, f)))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::builtin;
    use seiva_llm::{Message, Role, StopReason, Usage};
    use serde_json::json;

    fn masker() -> Masker {
        Masker::new(builtin())
    }

    #[test]
    fn the_same_value_gets_the_same_placeholder_and_numbers_follow_the_text() {
        let mut v = Vault::default();
        let t = masker()
            .mask_text(
                "BI 006045398LA041, email a@b.ao, outra vez 006045398LA041 e c@d.ao",
                &mut v,
                true,
            )
            .unwrap();
        assert_eq!(
            t,
            "BI ‹ID_CARD_1›, email ‹EMAIL_1›, outra vez ‹ID_CARD_1› e ‹EMAIL_2›"
        );
        assert_eq!(v.counts().get("EMAIL"), Some(&2));
        assert_eq!(
            v.unmask(&t),
            "BI 006045398LA041, email a@b.ao, outra vez 006045398LA041 e c@d.ao"
        );
    }

    #[test]
    fn a_resent_conversation_masks_to_the_same_placeholders() {
        let first = ChatRequest::new(10).user("O NIF 5417123456 pediu um estorno.");
        let second = first
            .clone()
            .message(Message::assistant("Certo."))
            .user("E o email ana@x.ao?");
        let (mut v1, mut v2) = (Vault::default(), Vault::default());
        let a = masker().mask_request(&first, &mut v1, true).unwrap();
        let b = masker().mask_request(&second, &mut v2, true).unwrap();
        assert_eq!(a.messages[0], b.messages[0]);
    }

    #[test]
    fn a_credential_stops_the_masking_when_refused() {
        let mut v = Vault::default();
        assert_eq!(
            masker().mask_text("usa sk-ant-api03-abcdefghijklmnopqrstuv", &mut v, true),
            Err(CredentialFound)
        );
        let masked = masker()
            .mask_text("usa sk-ant-api03-abcdefghijklmnopqrstuv", &mut v, false)
            .unwrap();
        assert_eq!(masked, "usa ‹CREDENTIAL_1›");
    }

    #[test]
    fn tool_results_and_tool_inputs_are_masked_too() {
        let req = ChatRequest::new(10).message(Message::new(
            Role::User,
            vec![
                Content::tool_result("t1", "{\"nif\": \"NIF 5417123456\"}", false),
                Content::ToolUse {
                    id: "t0".into(),
                    name: "lookup".into(),
                    input: json!({"email": "ana@x.ao", "n": 3}),
                },
            ],
        ));
        let mut v = Vault::default();
        let m = masker().mask_request(&req, &mut v, true).unwrap();
        let Content::ToolResult { content, .. } = &m.messages[0].content[0] else {
            unreachable!()
        };
        assert!(!content.contains("5417123456"));
        let Content::ToolUse { input, .. } = &m.messages[0].content[1] else {
            unreachable!()
        };
        assert_eq!(input, &json!({"email": "‹EMAIL_1›", "n": 3}));
    }

    #[test]
    fn unmasking_restores_text_and_tool_inputs_but_not_foreign_placeholders() {
        let mut v = Vault::default();
        masker().mask_text("ana@x.ao", &mut v, true).unwrap();
        let r = ChatResponse::new(
            "m",
            vec![
                Content::text("Enviei para ‹EMAIL_1› e ‹EMAIL_9›."),
                Content::ToolUse {
                    id: "t".into(),
                    name: "send".into(),
                    input: json!({"to": "‹EMAIL_1›"}),
                },
            ],
            StopReason::ToolUse,
            Usage::default(),
        );
        let u = unmask_response(&r, &v);
        assert_eq!(u.text(), "Enviei para ana@x.ao e ‹EMAIL_9›.");
        assert_eq!(u.tool_uses()[0].2, &json!({"to": "ana@x.ao"}));
    }

    #[test]
    fn the_vault_never_prints_a_value() {
        let mut v = Vault::default();
        masker().mask_text("ana@x.ao", &mut v, true).unwrap();
        assert!(!format!("{v:?}").contains("ana@x.ao"));
    }
}
