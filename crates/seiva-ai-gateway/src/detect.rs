//! What counts as sensitive: the [`Detector`] trait and the built-in detectors.
//!
//! A detector finds byte ranges in a text. The built-ins cover what a request to
//! a model abroad must never carry in clear — Angolan identity card and tax
//! numbers, IBANs (checked with mod-97), payment cards (checked with Luhn),
//! e-mail addresses, phone numbers — and credentials, which the gateway refuses
//! outright rather than masks (OWASP LLM02, LLM07). A product adds its own
//! identifiers with [`PatternDetector`].
//!
//! Detection is pattern-based: it finds what it knows and misses what it does
//! not. Masking reduces what a model sees; it does not make a prompt anonymous.

use std::ops::Range;
use std::sync::Arc;

use regex::Regex;

/// Finds sensitive values in a text.
pub trait Detector: Send + Sync {
    /// The placeholder label: `‹LABEL_1›`. Upper case, digits and `_`.
    fn label(&self) -> &str;

    /// Whether a match is a credential. Credentials are not masked and sent:
    /// by default the request is refused (OWASP LLM07).
    fn is_credential(&self) -> bool {
        false
    }

    /// The byte ranges of every match in `text`, on character boundaries.
    fn find(&self, text: &str) -> Vec<Range<usize>>;
}

/// A detector from a regular expression, optionally reading one capture group
/// and confirming each match with a validator (a checksum, say).
#[derive(Clone)]
pub struct PatternDetector {
    label: String,
    regex: Regex,
    group: usize,
    validate: Option<fn(&str) -> bool>,
    credential: bool,
}

impl PatternDetector {
    /// A detector labelled `label` matching `pattern` (the `regex` crate's
    /// syntax: no look-around).
    pub fn new(label: impl Into<String>, pattern: &str) -> Result<Self, regex::Error> {
        let label = label.into();
        debug_assert!(
            label
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
            "a label is upper case, digits and `_`"
        );
        Ok(Self {
            label,
            regex: Regex::new(pattern)?,
            group: 0,
            validate: None,
            credential: false,
        })
    }

    /// Mask only capture group `group` (the number after a `NIF:` prefix, say).
    #[must_use]
    pub fn with_group(mut self, group: usize) -> Self {
        self.group = group;
        self
    }

    /// Keep only the matches `validate` accepts.
    #[must_use]
    pub fn with_validator(mut self, validate: fn(&str) -> bool) -> Self {
        self.validate = Some(validate);
        self
    }

    /// Mark the matches as credentials.
    #[must_use]
    pub fn credential(mut self) -> Self {
        self.credential = true;
        self
    }
}

impl Detector for PatternDetector {
    fn label(&self) -> &str {
        &self.label
    }

    fn is_credential(&self) -> bool {
        self.credential
    }

    fn find(&self, text: &str) -> Vec<Range<usize>> {
        self.regex
            .captures_iter(text)
            .filter_map(|c| c.get(self.group))
            .filter(|m| self.validate.map_or(true, |v| v(m.as_str())))
            .map(|m| m.range())
            .collect()
    }
}

/// IBANs, confirmed with the mod-97 check. When a match runs into the text that
/// follows, the longest valid prefix is kept.
struct IbanDetector {
    regex: Regex,
}

impl Detector for IbanDetector {
    fn label(&self) -> &str {
        "IBAN"
    }

    fn find(&self, text: &str) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        for m in self.regex.find_iter(text) {
            let s = m.as_str();
            // Try every prefix that ends on an alphanumeric, longest first.
            let ends: Vec<usize> = s
                .char_indices()
                .filter(|(_, c)| c.is_ascii_alphanumeric())
                .map(|(i, c)| i + c.len_utf8())
                .collect();
            if let Some(end) = ends.into_iter().rev().find(|&end| iban_valid(&s[..end])) {
                out.push(m.start()..m.start() + end);
            }
        }
        out
    }
}

/// Whether `s` (spaces allowed) is an IBAN with a valid mod-97 check.
pub fn iban_valid(s: &str) -> bool {
    let compact: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if !(15..=34).contains(&compact.len()) || !compact.chars().all(|c| c.is_ascii_alphanumeric()) {
        return false;
    }
    let (head, tail) = compact.split_at(4);
    let mut rem: u32 = 0;
    for c in tail.chars().chain(head.chars()) {
        let v = c.to_digit(36).unwrap_or(0);
        rem = if v >= 10 {
            (rem * 100 + v) % 97
        } else {
            (rem * 10 + v) % 97
        };
    }
    rem == 1
}

/// Whether the digits of `s` (separators allowed) pass the Luhn check, with a
/// payment-card length.
pub fn luhn_valid(s: &str) -> bool {
    let digits: Vec<u32> = s.chars().filter_map(|c| c.to_digit(10)).collect();
    if !(13..=19).contains(&digits.len()) {
        return false;
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| {
            if i % 2 == 1 {
                let x = d * 2;
                if x > 9 {
                    x - 9
                } else {
                    x
                }
            } else {
                d
            }
        })
        .sum();
    sum % 10 == 0
}

/// The built-in detectors, in priority order: when two find the same text, the
/// earlier one names it.
pub fn builtin() -> Vec<Arc<dyn Detector>> {
    let credentials = [
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----",
        r"\bsk-ant-[A-Za-z0-9_\-]{20,}",
        r"\bsk-(?:proj-)?[A-Za-z0-9_\-]{20,}",
        r"\bAKIA[0-9A-Z]{16}\b",
        r"\bgh[pousr]_[A-Za-z0-9]{30,}",
        r"\bgithub_pat_[A-Za-z0-9_]{22,}",
        r"\bxox[baprs]-[A-Za-z0-9\-]{10,}",
        r"\bAIza[0-9A-Za-z_\-]{35}",
        r"\beyJ[A-Za-z0-9_\-]{8,}\.eyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}",
        // A connection string with a password: scheme://user:password@host
        r"\b[a-z][a-z0-9+.\-]{1,20}://[^\s:/@]+:[^\s@/]+@[^\s/]+",
    ]
    .join("|");
    let detectors: Vec<Arc<dyn Detector>> = vec![
        Arc::new(PatternDetector::new("CREDENTIAL", &credentials).expect("valid pattern").credential()),
        // NIF: 10 digits (a company) or the identity-card format (a person), after
        // the word NIF.
        Arc::new(
            PatternDetector::new(
                "TAX_ID",
                r"(?i)\bNIF\b(?:\s*(?:n\.?\s?º|nº|no\.?|n\.?|:|-))?\s*(\d{10}|\d{9}[A-Za-z]{2}\d{3})\b",
            )
            .expect("valid pattern")
            .with_group(1),
        ),
        Arc::new(IbanDetector {
            regex: Regex::new(r"\b[A-Z]{2}\d{2}(?: ?[A-Z0-9]{4}){2,7}(?: ?[A-Z0-9]{1,3})?\b").expect("valid pattern"),
        }),
        // The Angolan identity card: 9 digits, the 2-letter province, 3 digits.
        Arc::new(PatternDetector::new("ID_CARD", r"\b\d{9}[A-Z]{2}\d{3}\b").expect("valid pattern")),
        Arc::new(
            PatternDetector::new("CARD", r"\b(?:\d[ \-]?){12,18}\d\b")
                .expect("valid pattern")
                .with_validator(luhn_valid),
        ),
        Arc::new(
            PatternDetector::new("EMAIL", r"\b[A-Za-z0-9._%+\-]+@[A-Za-z0-9\-]+(?:\.[A-Za-z0-9\-]+)*\.[A-Za-z]{2,}\b")
                .expect("valid pattern"),
        ),
        // International (+244 923 456 789, 00351 …) and Angolan mobile (923 456 789).
        Arc::new(
            PatternDetector::new(
                "PHONE",
                r"(?:\+|\b00)\d{1,3}[ .\-]?\(?\d{1,4}\)?(?:[ .\-]?\d{2,4}){2,4}\b|\b9\d{2}[ .\-]?\d{3}[ .\-]?\d{3}\b",
            )
            .expect("valid pattern"),
        ),
    ];
    detectors
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(text: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for d in builtin() {
            for r in d.find(text) {
                out.push((d.label().to_owned(), text[r].to_owned()));
            }
        }
        out
    }

    fn found(text: &str, label: &str) -> Vec<String> {
        labels(text)
            .into_iter()
            .filter(|(l, _)| l == label)
            .map(|(_, v)| v)
            .collect()
    }

    #[test]
    fn the_angolan_identity_card_and_nif() {
        assert_eq!(
            found("BI 006045398LA041 emitido", "ID_CARD"),
            vec!["006045398LA041"]
        );
        assert_eq!(
            found("NIF: 5417123456 da empresa", "TAX_ID"),
            vec!["5417123456"]
        );
        assert_eq!(
            found("o NIF nº 006045398LA041", "TAX_ID"),
            vec!["006045398LA041"]
        );
        // Ten digits without the word NIF are not a tax id.
        assert!(found("pedido 5417123456", "TAX_ID").is_empty());
    }

    #[test]
    fn ibans_must_pass_mod_97() {
        // A valid Angolan IBAN (check digits computed for this BBAN).
        let iban = valid_ao_iban();
        assert!(iban_valid(&iban));
        assert_eq!(
            found(&format!("transferir para {iban}."), "IBAN"),
            vec![iban.clone()]
        );
        let spaced: String = iban
            .as_bytes()
            .chunks(4)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            found(&format!("IBAN {spaced} obrigado"), "IBAN"),
            vec![spaced]
        );
        assert!(
            !iban_valid("AO06000600000000000000001"),
            "a wrong check digit"
        );
        assert!(found("GB82WEST12345698765433", "IBAN").is_empty());
        assert_eq!(
            found("GB82 WEST 1234 5698 7654 32", "IBAN"),
            vec!["GB82 WEST 1234 5698 7654 32"]
        );
    }

    /// An `AO` IBAN with correct check digits for a fixed 21-digit BBAN.
    fn valid_ao_iban() -> String {
        let bban = "000600000123456789101";
        for check in 2..=98 {
            let candidate = format!("AO{check:02}{bban}");
            if iban_valid(&candidate) {
                return candidate;
            }
        }
        unreachable!()
    }

    #[test]
    fn cards_must_pass_luhn() {
        assert_eq!(
            found("cartão 4111 1111 1111 1111 ok", "CARD"),
            vec!["4111 1111 1111 1111"]
        );
        assert!(found("cartão 4111 1111 1111 1112", "CARD").is_empty());
    }

    #[test]
    fn emails_and_phones() {
        assert_eq!(
            found("escreve para ana.silva@exemplo.co.ao hoje", "EMAIL"),
            vec!["ana.silva@exemplo.co.ao"]
        );
        assert_eq!(
            found("liga +244 923 456 789 ou", "PHONE"),
            vec!["+244 923 456 789"]
        );
        assert_eq!(found("liga 923456789 amanhã", "PHONE"), vec!["923456789"]);
        // The identity card is not a phone number.
        assert!(found("006045398LA041", "PHONE").is_empty());
    }

    #[test]
    fn credentials_are_marked_as_such() {
        let d = builtin();
        assert!(d[0].is_credential());
        for text in [
            "chave sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123",
            "AKIAIOSFODNN7EXAMPLE",
            "postgres://app:s3cr3t@db.internal:5432/app",
            "-----BEGIN RSA PRIVATE KEY-----",
            "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
        ] {
            assert_eq!(found(text, "CREDENTIAL").len(), 1, "{text}");
        }
        assert!(found("https://example.com/path", "CREDENTIAL").is_empty());
    }

    #[test]
    fn a_product_adds_its_own_identifier() {
        let d = PatternDetector::new("CLIENT_ID", r"\bCLI-\d{6}\b").unwrap();
        assert_eq!(d.find("cliente CLI-004512 pediu"), vec![8..18]);
    }
}
