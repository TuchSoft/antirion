//! The built-in PII and secret detectors over stored content: every string of a JSON value is
//! searched with every entity and detector, and the matches are replaced. Values a request is
//! known to carry (those its reversible tokens stood for) are replaced too, under their label.

use serde_json::Value;

use crate::def::Replacement;
use crate::pattern::Span;

/// Values known to be personal data, each with the label its replacement names.
pub type Known = [(String, String)];

/// Every PII entity, secret and known value in `text`, merged.
#[must_use]
pub fn sensitive(text: &str, known: &Known) -> Vec<Span> {
    let spans: Vec<Span> = crate::pii::ALL
        .iter()
        .flat_map(|e| e.spans(text))
        .chain(crate::secrets::ALL.iter().flat_map(|d| d.spans(text)))
        .chain(
            known
                .iter()
                .filter(|(v, _)| !v.is_empty())
                .flat_map(|(v, label)| {
                    text.match_indices(v.as_str()).map(|(at, m)| Span {
                        start: at,
                        end: at + m.len(),
                        label: label.clone(),
                    })
                }),
        )
        .collect();
    crate::redact::merge(spans)
}

/// Replaces every match in the strings of `v`; true when anything matched.
pub fn scrub(v: &mut Value, how: Replacement, hash_key: Option<&[u8]>, known: &Known) -> bool {
    match v {
        Value::String(s) => {
            let spans = sensitive(s, known);
            if spans.is_empty() {
                return false;
            }
            *s = crate::redact::apply(
                s,
                &spans,
                crate::redact::Style {
                    how,
                    hash_key,
                    keep: &[],
                },
            );
            true
        }
        Value::Array(items) => {
            let mut found = false;
            for i in items {
                found |= scrub(i, how, hash_key, known);
            }
            found
        }
        Value::Object(m) => {
            let mut found = false;
            for i in m.values_mut() {
                found |= scrub(i, how, hash_key, known);
            }
            found
        }
        _ => false,
    }
}

/// Whether any string of `v` holds a match.
#[must_use]
pub fn holds_sensitive(v: &Value, known: &Known) -> bool {
    match v {
        Value::String(s) => !sensitive(s, known).is_empty(),
        Value::Array(items) => items.iter().any(|i| holds_sensitive(i, known)),
        Value::Object(m) => m.values().any(|i| holds_sensitive(i, known)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn strings_anywhere_in_a_value_are_scrubbed() {
        let mut v = json!({"messages": [{"role": "user", "content": "mail ada@example.com now"}], "n": 3});
        assert!(holds_sensitive(&v, &[]));
        assert!(scrub(&mut v, Replacement::Label, None, &[]));
        assert_eq!(v["messages"][0]["content"], "mail [EMAIL] now");
        assert!(!holds_sensitive(&v, &[]));
        let mut h = json!("key AKIAABCDEFGHIJKLMNOP here");
        assert!(scrub(&mut h, Replacement::Hash, Some(b"k"), &[]));
        assert!(h.as_str().unwrap().starts_with("key [AWS_ACCESS_KEY:"));
        assert!(!scrub(
            &mut json!({"a": "nothing"}),
            Replacement::Label,
            None,
            &[]
        ));
    }

    #[test]
    fn known_values_are_scrubbed_under_their_label() {
        let known = [("529.982.247-25".to_owned(), "CPF".to_owned())];
        let mut v = json!({"content": "your CPF \"529.982.247-25\" is on file"});
        assert!(!holds_sensitive(&v, &[]));
        assert!(holds_sensitive(&v, &known));
        assert!(scrub(&mut v, Replacement::Label, None, &known));
        assert_eq!(v["content"], "your CPF \"[CPF]\" is on file");
    }
}
