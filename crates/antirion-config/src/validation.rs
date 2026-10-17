//! The whole validation of a resulting configuration: schema, redacted secrets, semantic rules.

use serde_json::Value;

use crate::{ConfigError, DocPath, schema, secrets, semantic};

/// Everything validation found, plus where the document's secret fields are.
#[derive(Debug, Default)]
pub struct Validation {
    pub errors: Vec<ConfigError>,
    pub warnings: Vec<ConfigError>,
    pub secret_paths: Vec<DocPath>,
}

impl Validation {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Validates `doc` as a whole configuration. Secret fields written as [`secrets::REDACTED`] take the
/// value stored at the same path of `previous`, in place.
pub fn validate(doc: &mut Value, previous: &Value) -> Validation {
    let check = schema::check_document(doc);
    let mut errors = check.errors;
    errors.extend(secrets::restore_redacted(doc, previous, &check.secret_paths));
    errors.extend(semantic::check(doc));
    let mut seen = std::collections::HashSet::new();
    errors.retain(|e| seen.insert((e.path.clone(), e.code, e.message.clone())));
    errors.sort_by(|a, b| a.path.cmp(&b.path));
    Validation {
        errors,
        warnings: Vec::new(),
        secret_paths: check.secret_paths,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::DetailCode;

    #[test]
    fn three_independent_errors() {
        let mut doc = json!({
            "version": 1,
            "providers": {"p": {"type": "openai", "base_url": "http://127.0.0.1:9/v1",
                                "auth": {"method": "api_key", "api_key": "sk-x"}}},
            "deployments": {"p": {"provider": "p", "model": "up"}, "second": {"provider": "p", "model": "o"}},
            "models": {"m": {"target": {"deployment": "p"}}, "m2": {"target": {"deployment": "ghost"}}},
            "routes": {"r": {"tree": {"type": "weighted", "children": [
                {"deployment": "p", "weight": 1}, {"deployment": "second", "weight": -1}]}}},
            "policies": {"resilience": {"timeouts": {"connect": "10"}}}
        });
        let v = validate(&mut doc, &json!({}));
        let got: Vec<(String, DetailCode)> = v.errors.into_iter().map(|e| (e.path, e.code)).collect();
        assert_eq!(
            got,
            vec![
                (
                    "models.m2.target.deployment".to_owned(),
                    DetailCode::UnresolvedReference
                ),
                (
                    "policies.resilience.timeouts.connect".to_owned(),
                    DetailCode::InvalidFormat
                ),
                (
                    "routes.r.tree.children[1].weight".to_owned(),
                    DetailCode::OutOfRange
                ),
            ]
        );
    }

    #[test]
    fn a_saved_single_key_follows_the_provider_into_a_key_list() {
        let provider = |auth: serde_json::Value| json!({"version": 1, "providers": {"p": {"type": "openai", "base_url": "http://127.0.0.1:9/v1", "auth": auth}}});
        let stored = provider(json!({"api_key": "sk-saved"}));
        let mut doc = provider(json!({"api_keys": [
            {"name": "default", "key": crate::secrets::REDACTED}, {"name": "new", "key": "sk-new"}
        ]}));
        let v = validate(&mut doc, &stored);
        assert!(v.errors.is_empty(), "{:?}", v.errors);
        assert_eq!(
            doc.pointer("/providers/p/auth/api_keys/0/key"),
            Some(&json!("sk-saved"))
        );
        // Removing the first of two saved keys leaves the second with its own secret.
        let stored = doc.clone();
        let mut doc = provider(json!({"api_keys": [{"name": "new", "key": crate::secrets::REDACTED}]}));
        let v = validate(&mut doc, &stored);
        assert!(v.errors.is_empty(), "{:?}", v.errors);
        assert_eq!(
            doc.pointer("/providers/p/auth/api_keys/0/key"),
            Some(&json!("sk-new"))
        );
    }
}
