//! Secret references (`${env:NAME}`, `${file:/abs/path}` and the external stores'
//! `${<store>:<reference>}`), `$$` escapes and redaction.

use std::borrow::Cow;

use serde_json::Value;

use crate::path::Seg;
use crate::{ConfigError, DetailCode, DocPath};

/// What reads and exports show instead of a literal secret; written back, it keeps the stored value.
pub const REDACTED: &str = "[REDACTED]";

/// The external secret stores a reference can name.
pub const STORE_SCHEMES: [&str; 4] = ["vault", "aws-sm", "gcp-sm", "azure-kv"];

/// Where references are resolved from. Tests inject their own instead of touching the process
/// environment.
pub trait SecretSource: Send + Sync + std::fmt::Debug {
    fn env(&self, name: &str) -> Option<String>;
    fn file(&self, path: &str) -> std::io::Result<String>;

    /// The value of `${<store>:<reference>}` as the store last answered it, or why there is none
    /// (never a value).
    fn store(&self, store: &str, reference: &str) -> Result<String, String> {
        let _ = (store, reference);
        Err("no secret store of that name is configured".to_owned())
    }
}

/// The process environment and the local filesystem.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemSecrets;

impl SecretSource for SystemSecrets {
    fn env(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    fn file(&self, path: &str) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }
}

/// Why a reference did not resolve. The message names the reference, never a value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SecretError(pub String);

/// Expands every reference in `text` in a single pass: `$$` is `$`, `${env:…}` and `${file:…}` are
/// replaced by their values (one trailing newline of a file is removed), and every other `$` form is
/// literal. Resolved values are never scanned again.
pub fn expand<'a>(text: &'a str, source: &dyn SecretSource) -> Result<Cow<'a, str>, SecretError> {
    if !text.contains('$') {
        return Ok(Cow::Borrowed(text));
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        if let Some(after) = tail.strip_prefix("$$") {
            out.push('$');
            rest = after;
        } else if let Some((reference, after)) = reference_at(tail) {
            out.push_str(&resolve(reference, source)?);
            rest = after;
        } else {
            out.push('$');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    Ok(Cow::Owned(out))
}

/// The reference starting at `s` (`${scheme:rest}`) and the text after it.
fn reference_at(s: &str) -> Option<(&str, &str)> {
    let body = s.strip_prefix("${")?;
    let end = body.find('}')?;
    let inner = &body[..end];
    let (scheme, rest) = inner.split_once(':')?;
    let valid_scheme = !scheme.is_empty()
        && scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    (valid_scheme && !rest.is_empty()).then(|| (inner, &body[end + 1..]))
}

fn resolve(reference: &str, source: &dyn SecretSource) -> Result<String, SecretError> {
    match reference.split_once(':') {
        Some(("env", name)) => source
            .env(name)
            .ok_or_else(|| SecretError(format!("${{{reference}}}: environment variable is not set"))),
        Some(("file", path)) => {
            let mut value = source
                .file(path)
                .map_err(|_| SecretError(format!("${{{reference}}}: the file cannot be read")))?;
            if value.ends_with("\r\n") {
                value.truncate(value.len() - 2);
            } else if value.ends_with('\n') {
                value.truncate(value.len() - 1);
            }
            Ok(value)
        }
        Some((store, rest)) if STORE_SCHEMES.contains(&store) => source
            .store(store, rest)
            .map_err(|why| SecretError(format!("${{{reference}}}: {why}"))),
        _ => Err(SecretError(format!(
            "${{{reference}}}: no secret store of that name is configured"
        ))),
    }
}

/// Every reference in `text` (`scheme:rest`, without `${` and `}`), skipping `$$` escapes.
fn references(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find('$') {
        let tail = &rest[at..];
        if let Some(after) = tail.strip_prefix("$$") {
            rest = after;
        } else if let Some((reference, after)) = reference_at(tail) {
            out.push(reference);
            rest = after;
        } else {
            rest = &tail[1..];
        }
    }
    out
}

/// Every external-store reference in the string values of `v`, as `(store, reference)`, in
/// document order and without repeats.
#[must_use]
pub fn store_references(v: &Value) -> Vec<(String, String)> {
    fn walk(v: &Value, out: &mut Vec<(String, String)>) {
        match v {
            Value::String(s) => {
                for r in references(s) {
                    if let Some((store, rest)) = r.split_once(':')
                        && STORE_SCHEMES.contains(&store)
                        && !out.iter().any(|(a, b)| a == store && b == rest)
                    {
                        out.push((store.to_owned(), rest.to_owned()));
                    }
                }
            }
            Value::Array(items) => items.iter().for_each(|x| walk(x, out)),
            Value::Object(map) => map.values().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(v, &mut out);
    out
}

/// True when `text` reads the node's environment or a file (`${env:…}`, `${file:…}`).
#[must_use]
pub fn reads_host(text: &str) -> bool {
    references(text)
        .iter()
        .any(|r| r.starts_with("env:") || r.starts_with("file:"))
}

/// True when `text` holds at least one reference; such values are shown as written on read paths.
#[must_use]
pub fn has_reference(text: &str) -> bool {
    !references(text).is_empty()
}

/// Expands every string value of `doc` (never keys); each failure is `secret_unresolved` at its path.
pub fn expand_document(doc: &Value, source: &dyn SecretSource) -> Result<Value, Vec<ConfigError>> {
    let mut errors = Vec::new();
    let mut path = DocPath::root();
    let out = expand_value(doc, source, &mut path, &mut errors);
    if errors.is_empty() { Ok(out) } else { Err(errors) }
}

fn expand_value(
    v: &Value,
    source: &dyn SecretSource,
    path: &mut DocPath,
    errors: &mut Vec<ConfigError>,
) -> Value {
    match v {
        Value::String(s) => match expand(s, source) {
            Ok(Cow::Borrowed(_)) => v.clone(),
            Ok(Cow::Owned(o)) => Value::String(o),
            Err(e) => {
                errors.push(ConfigError::new(path, DetailCode::SecretUnresolved, e.0));
                v.clone()
            }
        },
        Value::Array(items) => Value::Array(
            items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    path.push_index(i);
                    let out = expand_value(item, source, path, errors);
                    path.pop();
                    out
                })
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, item)| {
                    path.push_key(k);
                    let out = expand_value(item, source, path, errors);
                    path.pop();
                    (k.clone(), out)
                })
                .collect(),
        ),
        _ => v.clone(),
    }
}

/// A copy of `doc` with every literal at a secret path replaced by [`REDACTED`].
#[must_use]
pub fn redact(doc: &Value, secret_paths: &[DocPath]) -> Value {
    let mut out = doc.clone();
    for path in secret_paths {
        if let Some(slot) = path.get_mut(&mut out)
            && let Value::String(s) = slot
            && !has_reference(s)
        {
            *slot = Value::String(REDACTED.to_owned());
        }
    }
    out
}

/// The members that name a list item: a provider key's `name`, an MCP session key's `id`.
const ITEM_IDS: [&str; 2] = ["name", "id"];

/// The name of the single provider key, `auth.api_key`, among `auth.api_keys`.
const SINGLE_KEY: &str = "default";

/// What names a list item, when it has a name.
fn item_id(item: &Value) -> Option<(&'static str, &Value)> {
    ITEM_IDS.iter().find_map(|k| {
        item.get(*k)
            .filter(|v| v.is_string() || v.is_number())
            .map(|v| (*k, v))
    })
}

/// The stored value a secret at `path` of `doc` stands for in `previous`. A named list item is
/// found by its name, whatever its position: removing or reordering items keeps every secret with
/// its item. A list of provider keys that replaced a single `auth.api_key` finds that key under the
/// item named `default`, or under the first item when no item is named so.
fn stored_at<'a>(path: &DocPath, doc: &Value, previous: &'a Value) -> Option<&'a Value> {
    let segments = path.segments();
    let mut here = doc;
    let mut there = Some(previous);
    for (at, seg) in segments.iter().enumerate() {
        match seg {
            Seg::Key(k) => {
                here = here.get(k)?;
                there = there.and_then(|t| t.get(k));
            }
            Seg::Index(i) => {
                let items = here.as_array()?;
                let item = items.get(*i)?;
                let rest = segments.get(at + 1..).unwrap_or_default();
                here = item;
                there = match (item_id(item), there.and_then(Value::as_array)) {
                    (None, Some(stored)) => stored.get(*i),
                    (Some((field, id)), Some(stored)) => stored.iter().find(|s| s.get(field) == Some(id)),
                    (_, None) => return single_key(segments.get(..at)?, previous, items, *i, rest),
                };
            }
        }
    }
    there
}

/// The stored `auth.api_key` that the `index`th item of a new `auth.api_keys` list stands for.
fn single_key<'a>(
    list: &[Seg],
    previous: &'a Value,
    items: &[Value],
    index: usize,
    rest: &[Seg],
) -> Option<&'a Value> {
    let (Some(Seg::Key(last)), [Seg::Key(field)]) = (list.last(), rest) else {
        return None;
    };
    if last != "api_keys" || field != "key" {
        return None;
    }
    let named = |item: &Value| item.get("name").and_then(Value::as_str) == Some(SINGLE_KEY);
    let ours = match items.iter().position(named) {
        Some(p) => p == index,
        None => index == 0,
    };
    if !ours {
        return None;
    }
    let mut auth = DocPath::root();
    for seg in list.get(..list.len() - 1)? {
        match seg {
            Seg::Key(k) => auth.push_key(k),
            Seg::Index(i) => auth.push_index(*i),
        }
    }
    auth.get(previous)?.get("api_key")
}

/// Replaces [`REDACTED`] at each secret path of `doc` with the value stored for it in `previous`
/// (the same path, or the same named list item); a placeholder without a stored value is
/// `secret_required`.
pub fn restore_redacted(doc: &mut Value, previous: &Value, secret_paths: &[DocPath]) -> Vec<ConfigError> {
    let mut errors = Vec::new();
    for path in secret_paths {
        if path.get(doc).and_then(Value::as_str) != Some(REDACTED) {
            continue;
        }
        let stored = stored_at(path, doc, previous).cloned();
        let Some(slot) = path.get_mut(doc) else { continue };
        match stored {
            Some(Value::String(stored)) if stored != REDACTED => *slot = Value::String(stored),
            _ => errors.push(ConfigError::new(
                path,
                DetailCode::SecretRequired,
                "the secret is redacted and no stored value exists; supply it",
            )),
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::*;

    fn key_paths(n: usize) -> Vec<DocPath> {
        let list = DocPath::root()
            .key("providers")
            .key("p")
            .key("auth")
            .key("api_keys");
        (0..n).map(|i| list.index(i).key("key")).collect()
    }

    fn keys(doc: &Value) -> Vec<&str> {
        doc.pointer("/providers/p/auth/api_keys")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|k| k["key"].as_str()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn redacted_provider_keys_are_restored_by_name() {
        let stored = json!({"providers": {"p": {"auth": {"api_keys": [
            {"name": "a", "key": "sk-a"}, {"name": "b", "key": "sk-b"}, {"name": "c", "key": "sk-c"}
        ]}}}});
        // The middle key removed: the later ones keep their own secrets.
        let mut doc = json!({"providers": {"p": {"auth": {"api_keys": [
            {"name": "a", "key": REDACTED}, {"name": "c", "key": REDACTED}
        ]}}}});
        assert!(restore_redacted(&mut doc, &stored, &key_paths(2)).is_empty());
        assert_eq!(keys(&doc), ["sk-a", "sk-c"]);
        // Reordered, with a new literal key added in front.
        let mut doc = json!({"providers": {"p": {"auth": {"api_keys": [
            {"name": "new", "key": "sk-new"}, {"name": "c", "key": REDACTED}, {"name": "a", "key": REDACTED}
        ]}}}});
        assert!(restore_redacted(&mut doc, &stored, &key_paths(3)).is_empty());
        assert_eq!(keys(&doc), ["sk-new", "sk-c", "sk-a"]);
        // A placeholder for a name that was never stored is not some other key's secret.
        let mut doc = json!({"providers": {"p": {"auth": {"api_keys": [
            {"name": "a", "key": REDACTED}, {"name": "d", "key": REDACTED}
        ]}}}});
        let errors = restore_redacted(&mut doc, &stored, &key_paths(2));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].code, DetailCode::SecretRequired);
        assert_eq!(errors[0].path, "providers.p.auth.api_keys[1].key");
        assert_eq!(
            doc.pointer("/providers/p/auth/api_keys/1/key"),
            Some(&json!(REDACTED))
        );
    }

    #[test]
    fn a_single_provider_key_becomes_the_default_item_of_a_list() {
        let stored = json!({"providers": {"p": {"auth": {"api_key": "sk-single"}}}});
        let mut doc = json!({"providers": {"p": {"auth": {"api_keys": [
            {"name": "extra", "key": "sk-extra"}, {"name": "default", "key": REDACTED}
        ]}}}});
        assert!(restore_redacted(&mut doc, &stored, &key_paths(2)).is_empty());
        assert_eq!(keys(&doc), ["sk-extra", "sk-single"]);
        // With no item named `default`, the first item is the saved key.
        let mut doc = json!({"providers": {"p": {"auth": {"api_keys": [
            {"name": "primary", "key": REDACTED}, {"name": "extra", "key": "sk-extra"}
        ]}}}});
        assert!(restore_redacted(&mut doc, &stored, &key_paths(2)).is_empty());
        assert_eq!(keys(&doc), ["sk-single", "sk-extra"]);
        // Any other item has nothing stored.
        let mut doc = json!({"providers": {"p": {"auth": {"api_keys": [
            {"name": "primary", "key": "sk-new"}, {"name": "extra", "key": REDACTED}
        ]}}}});
        assert_eq!(restore_redacted(&mut doc, &stored, &key_paths(2)).len(), 1);
    }

    #[test]
    fn mcp_session_keys_are_restored_by_id_and_unnamed_lists_by_position() {
        let stored = json!({
            "mcp": {"session": {"keys": [{"id": 1, "secret": "one"}, {"id": 2, "secret": "two"}]}},
            "security": {"secrets": {"encryption_keys": ["k0", "k1"]}}
        });
        let mut doc = json!({
            "mcp": {"session": {"keys": [{"id": 2, "secret": REDACTED}]}},
            "security": {"secrets": {"encryption_keys": [REDACTED, REDACTED]}}
        });
        let enc = DocPath::root()
            .key("security")
            .key("secrets")
            .key("encryption_keys");
        let paths = [
            DocPath::root()
                .key("mcp")
                .key("session")
                .key("keys")
                .index(0)
                .key("secret"),
            enc.index(0),
            enc.index(1),
        ];
        assert!(restore_redacted(&mut doc, &stored, &paths).is_empty());
        assert_eq!(doc.pointer("/mcp/session/keys/0/secret"), Some(&json!("two")));
        assert_eq!(
            doc.pointer("/security/secrets/encryption_keys"),
            Some(&json!(["k0", "k1"]))
        );
    }

    #[test]
    fn host_reads_are_env_and_file_references() {
        assert!(reads_host("${env:TOKEN}"));
        assert!(reads_host("Bearer ${file:/etc/token}"));
        assert!(!reads_host("${vault:secret/x#k}"));
        assert!(!reads_host("$${env:TOKEN}"));
        assert!(!reads_host("plain"));
    }

    #[derive(Debug)]
    struct Fake(HashMap<&'static str, &'static str>);

    impl SecretSource for Fake {
        fn env(&self, name: &str) -> Option<String> {
            self.0.get(name).map(|s| (*s).to_owned())
        }
        fn file(&self, path: &str) -> std::io::Result<String> {
            self.0
                .get(path)
                .map(|s| (*s).to_owned())
                .ok_or_else(|| std::io::Error::other("missing"))
        }
    }

    fn fake() -> Fake {
        Fake(HashMap::from([
            ("K", "sk-a$b${env:X}c"),
            ("/run/key", "secret\n"),
            ("X", "expanded"),
        ]))
    }

    #[test]
    fn escapes_and_plain_dollars() {
        let f = fake();
        assert_eq!(expand("a$$b", &f).unwrap(), "a$b");
        assert_eq!(expand("price $5", &f).unwrap(), "price $5");
        assert_eq!(expand("${HOME}", &f).unwrap(), "${HOME}");
        assert_eq!(expand("$HOME", &f).unwrap(), "$HOME");
        assert_eq!(expand("$${env:PATH}", &f).unwrap(), "${env:PATH}");
        assert_eq!(
            expand("sk-$$lit-$HOME-${HOME}-$", &f).unwrap(),
            "sk-$lit-$HOME-${HOME}-$"
        );
    }

    #[test]
    fn references_resolve_once() {
        let f = fake();
        assert_eq!(expand("${env:K}", &f).unwrap(), "sk-a$b${env:X}c");
        assert_eq!(expand("Bearer ${file:/run/key}", &f).unwrap(), "Bearer secret");
    }

    #[test]
    fn failures_name_the_reference() {
        let f = fake();
        assert!(
            expand("${env:MISSING}", &f)
                .unwrap_err()
                .0
                .contains("env:MISSING")
        );
        assert!(expand("${file:/nope}", &f).unwrap_err().0.contains("/nope"));
        assert!(expand("${vault:x}", &f).is_err());
        assert!(
            expand("${aws-sm:arn:aws:secretsmanager:us-east-1:1:secret:k}", &f)
                .unwrap_err()
                .0
                .contains("no secret store")
        );
    }

    #[derive(Debug)]
    struct Stores;

    impl SecretSource for Stores {
        fn env(&self, _: &str) -> Option<String> {
            None
        }
        fn file(&self, _: &str) -> std::io::Result<String> {
            Err(std::io::Error::other("none"))
        }
        fn store(&self, store: &str, reference: &str) -> Result<String, String> {
            match (store, reference) {
                ("vault", "secret/data/a#k") => Ok("v-${env:X}".to_owned()),
                _ => Err("the store refused the reference".to_owned()),
            }
        }
    }

    #[test]
    fn store_references_resolve_through_the_source() {
        assert_eq!(
            expand("Bearer ${vault:secret/data/a#k}", &Stores).unwrap(),
            "Bearer v-${env:X}"
        );
        let err = expand("${gcp-sm:projects/p/secrets/s}", &Stores).unwrap_err().0;
        assert!(
            err.contains("gcp-sm:projects/p/secrets/s") && err.contains("refused"),
            "{err}"
        );
        assert!(has_reference("${azure-kv:v/n}"));
        let doc = json!({"a": ["${vault:p#f}", "$${vault:escaped#f}"], "b": {"c": "x${aws-sm:k}y${vault:p#f}${env:E}"}});
        assert_eq!(
            store_references(&doc),
            [
                ("vault".to_owned(), "p#f".to_owned()),
                ("aws-sm".to_owned(), "k".to_owned())
            ]
        );
    }

    #[test]
    fn document_expansion_reports_paths() {
        let doc = json!({"providers": {"p": {"auth": {"api_key": "${env:MISSING}"}}}});
        let errs = expand_document(&doc, &fake()).unwrap_err();
        assert_eq!(errs[0].path, "providers.p.auth.api_key");
        assert_eq!(errs[0].code, DetailCode::SecretUnresolved);
    }

    #[test]
    fn redaction_and_restoration() {
        let paths = vec![
            DocPath::parse_dotted("p.auth.api_key"),
            DocPath::parse_dotted("q.auth.api_key"),
        ];
        let doc = json!({"p": {"auth": {"api_key": "sk-live"}}, "q": {"auth": {"api_key": "${env:K}"}}});
        let shown = redact(&doc, &paths);
        assert_eq!(shown["p"]["auth"]["api_key"], REDACTED);
        assert_eq!(shown["q"]["auth"]["api_key"], "${env:K}");
        let mut back = shown.clone();
        assert!(restore_redacted(&mut back, &doc, &paths).is_empty());
        assert_eq!(back, doc);
        let mut fresh = shown;
        let errs = restore_redacted(&mut fresh, &json!({}), &paths);
        assert_eq!(errs[0].code, DetailCode::SecretRequired);
    }
}
