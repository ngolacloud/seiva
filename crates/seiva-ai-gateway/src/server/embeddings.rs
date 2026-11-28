//! The OpenAI Embeddings shape: `POST /v1/embeddings`.
//!
//! The `model` field names a gateway **embedding route**. `input` is a string or
//! an array of strings; token arrays are refused, because the gateway masks text
//! and cannot read tokens. The vectors come back as JSON numbers, or — when the
//! client asks for `encoding_format: "base64"`, as the official OpenAI SDKs do by
//! default — as little-endian `f32` bytes in base64.

use seiva_llm::Embeddings;
use serde_json::{json, Value};

/// The most texts one request may carry (OpenAI's own limit).
pub(crate) const MAX_INPUTS: usize = 2048;

/// How the vectors are written back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Encoding {
    Float,
    Base64,
}

/// A parsed embeddings request.
#[derive(Debug)]
pub(crate) struct Parsed {
    pub route: String,
    pub inputs: Vec<String>,
    pub encoding: Encoding,
}

/// Read an embeddings request.
pub(crate) fn parse(body: &Value) -> Result<Parsed, String> {
    let route = body
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .ok_or("`model` (the gateway route) is required")?
        .to_owned();
    let inputs = match body.get("input") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(items)) if items.is_empty() => {
            return Err("`input` must not be empty".into())
        }
        Some(Value::Array(items)) => items
            .iter()
            .map(|i| match i {
                Value::String(s) => Ok(s.clone()),
                _ => Err(
                    "`input` must be a string or an array of strings: token arrays are not \
                     accepted (the gateway masks text, and cannot read tokens)"
                        .to_owned(),
                ),
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err("`input` (a string or an array of strings) is required".into()),
    };
    if inputs.len() > MAX_INPUTS {
        return Err(format!(
            "`input` has {} texts; the limit is {MAX_INPUTS}",
            inputs.len()
        ));
    }
    if inputs.iter().any(|t| t.is_empty()) {
        return Err("an `input` text is empty".into());
    }
    let encoding = match body.get("encoding_format").and_then(Value::as_str) {
        None | Some("float") => Encoding::Float,
        Some("base64") => Encoding::Base64,
        Some(other) => {
            return Err(format!(
                "`encoding_format` must be `float` or `base64`, not `{other}`"
            ))
        }
    };
    if body.get("dimensions").is_some_and(|d| !d.is_null()) {
        return Err(
            "`dimensions` is not accepted: the route decides the model and its dimension".into(),
        );
    }
    Ok(Parsed {
        route,
        inputs,
        encoding,
    })
}

/// The response body.
pub(crate) fn render(route: &str, embeddings: &Embeddings, encoding: Encoding) -> Value {
    let data: Vec<Value> = embeddings
        .vectors
        .iter()
        .enumerate()
        .map(|(index, v)| {
            let embedding = match encoding {
                Encoding::Float => json!(v),
                Encoding::Base64 => Value::String(base64_f32(v)),
            };
            json!({"object": "embedding", "index": index, "embedding": embedding})
        })
        .collect();
    let tokens = embeddings.usage.total_input();
    json!({
        "object": "list",
        "data": data,
        "model": route,
        "usage": {"prompt_tokens": tokens, "total_tokens": tokens},
    })
}

/// `v` as little-endian `f32` bytes, in standard base64 with padding — what the
/// OpenAI SDKs decode.
fn base64_f32(v: &[f32]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use seiva_llm::Usage;

    #[test]
    fn input_is_a_string_or_strings_never_tokens() {
        let p = parse(&json!({"model": "docs-embed", "input": "olá"})).unwrap();
        assert_eq!(p.inputs, vec!["olá"]);
        assert_eq!(p.encoding, Encoding::Float);
        let p = parse(&json!({"model": "e", "input": ["a", "b"], "encoding_format": "base64"}))
            .unwrap();
        assert_eq!((p.inputs.len(), p.encoding), (2, Encoding::Base64));
        for bad in [
            json!({"input": "a"}),
            json!({"model": "e"}),
            json!({"model": "e", "input": []}),
            json!({"model": "e", "input": [1, 2, 3]}),
            json!({"model": "e", "input": [[1, 2]]}),
            json!({"model": "e", "input": ["a", ""]}),
            json!({"model": "e", "input": "a", "encoding_format": "hex"}),
            json!({"model": "e", "input": "a", "dimensions": 256}),
        ] {
            assert!(parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn base64_is_little_endian_f32_with_padding() {
        // 1.0f32 = 00 00 80 3f.
        assert_eq!(base64_f32(&[1.0]), "AACAPw==");
        // Two floats: 8 bytes → 12 characters, one `=`.
        assert_eq!(base64_f32(&[1.0, -2.0]), "AACAPwAAAMA=");
        assert_eq!(base64_f32(&[]), "");
    }

    #[test]
    fn the_response_has_the_openai_shape() {
        let e = Embeddings::new(
            vec![vec![0.5, 0.25]],
            Usage {
                input_tokens: 4,
                ..Usage::default()
            },
        );
        let v = render("docs-embed", &e, Encoding::Float);
        assert_eq!(v["data"][0]["embedding"], json!([0.5, 0.25]));
        assert_eq!(v["data"][0]["index"], 0);
        assert_eq!(v["model"], "docs-embed");
        assert_eq!(v["usage"]["prompt_tokens"], 4);
    }
}
