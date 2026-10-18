//! What an OpenAI embeddings answer holds, read without its vectors: the usage, the model and the
//! shape of the vectors (how many, of how many dimensions), which is what the request log keeps of
//! the answer. A vector is skipped element by element, never collected, so reading a large answer
//! costs no more memory than the bytes already held.

use serde::Deserialize;
use serde::de::{Deserializer, IgnoredAny, SeqAccess, Visitor};
use serde_json::{Value, json};

/// One vector, by its length.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Vector {
    /// A list of numbers.
    Floats(u64),
    /// A base64 string of little-endian 32-bit floats (`encoding_format: base64`).
    Base64(u64),
}

impl Vector {
    fn dimensions(self) -> u64 {
        match self {
            Vector::Floats(n) | Vector::Base64(n) => n,
        }
    }
}

/// The floats a base64 string of `len` characters (padding included) decodes to.
fn base64_floats(s: &str) -> u64 {
    let chars = s.trim_end_matches('=').len() as u64;
    chars.saturating_mul(3) / 4 / 4
}

impl<'de> Deserialize<'de> for Vector {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Vector, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Vector;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an embedding vector")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vector, A::Error> {
                let mut n = 0u64;
                while seq.next_element::<IgnoredAny>()?.is_some() {
                    n += 1;
                }
                Ok(Vector::Floats(n))
            }

            fn visit_str<E: serde::de::Error>(self, s: &str) -> Result<Vector, E> {
                Ok(Vector::Base64(base64_floats(s)))
            }
        }
        d.deserialize_any(V)
    }
}

#[derive(Debug, Deserialize)]
struct Item {
    #[serde(default)]
    embedding: Option<Vector>,
}

/// An embeddings answer, its vectors reduced to their lengths.
#[derive(Debug, Deserialize)]
pub(crate) struct Embeddings {
    #[serde(default)]
    object: Option<String>,
    data: Vec<Item>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    usage: Option<Value>,
}

impl Embeddings {
    /// Reads an answer; `None` when it is not an embeddings list.
    pub(crate) fn read(raw: &[u8]) -> Option<Embeddings> {
        serde_json::from_slice(raw).ok()
    }

    /// The members the record's usage and response facts read: `usage`, `model` and `id`.
    pub(crate) fn facts(&self) -> Value {
        let mut v = serde_json::Map::new();
        if let Some(u) = &self.usage {
            v.insert("usage".into(), u.clone());
        }
        if let Some(m) = &self.model {
            v.insert("model".into(), Value::from(m.as_str()));
        }
        if let Some(id) = &self.id {
            v.insert("id".into(), Value::from(id.as_str()));
        }
        Value::Object(v)
    }

    /// The answer as the request log keeps it: its members, with `data` replaced by the number of
    /// vectors, their dimensions (one number when they agree, else one per vector) and their
    /// encoding.
    pub(crate) fn summary(&self) -> Value {
        let mut dims: Vec<u64> = self
            .data
            .iter()
            .filter_map(|i| i.embedding.map(Vector::dimensions))
            .collect();
        let uniform = dims.windows(2).all(|w| w.first() == w.last());
        let dimensions = if uniform {
            dims.first().map_or(Value::Null, |d| Value::from(*d))
        } else {
            Value::from(std::mem::take(&mut dims))
        };
        let encoding = match self.data.iter().find_map(|i| i.embedding) {
            Some(Vector::Base64(_)) => "base64",
            _ => "float",
        };
        let mut out = serde_json::Map::new();
        out.insert(
            "object".into(),
            Value::from(self.object.as_deref().unwrap_or("list")),
        );
        if let Some(m) = &self.model {
            out.insert("model".into(), Value::from(m.as_str()));
        }
        out.insert(
            "data".into(),
            json!({
                "vectors": self.data.len(),
                "dimensions": dimensions,
                "encoding_format": encoding,
                "omitted": true,
            }),
        );
        if let Some(u) = &self.usage {
            out.insert("usage".into(), u.clone());
        }
        Value::Object(out)
    }

    /// A line for the `gen_ai.output.messages` span attribute.
    pub(crate) fn describe(&self) -> String {
        let dims = self
            .data
            .first()
            .and_then(|i| i.embedding)
            .map_or(0, Vector::dimensions);
        format!("{} embeddings of {dims} dimensions", self.data.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_are_counted_not_kept() {
        let raw = br#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.1,0.2,0.3]},
            {"object":"embedding","index":1,"embedding":[1,2,3]}],"model":"m","usage":{"prompt_tokens":2,"total_tokens":2}}"#;
        let e = Embeddings::read(raw).unwrap();
        assert_eq!(
            e.summary(),
            json!({"object": "list", "model": "m",
                "data": {"vectors": 2, "dimensions": 3, "encoding_format": "float", "omitted": true},
                "usage": {"prompt_tokens": 2, "total_tokens": 2}})
        );
        assert_eq!(e.facts()["usage"]["prompt_tokens"], 2);
        assert_eq!(e.describe(), "2 embeddings of 3 dimensions");
    }

    #[test]
    fn base64_vectors_report_their_floats() {
        // Three floats: twelve bytes, sixteen base64 characters.
        let raw = br#"{"data":[{"embedding":"AACAPwAAAEAAAEBA"}],"usage":{"prompt_tokens":1}}"#;
        let s = Embeddings::read(raw).unwrap().summary();
        assert_eq!(s["data"]["dimensions"], 3);
        assert_eq!(s["data"]["encoding_format"], "base64");
    }

    #[test]
    fn uneven_vectors_list_each_length_and_other_bodies_are_not_read() {
        let raw = br#"{"data":[{"embedding":[1]},{"embedding":[1,2]}]}"#;
        assert_eq!(
            Embeddings::read(raw).unwrap().summary()["data"]["dimensions"],
            json!([1, 2])
        );
        assert!(Embeddings::read(br#"{"error":{"message":"x"}}"#).is_none());
        assert!(Embeddings::read(b"not json").is_none());
    }
}
