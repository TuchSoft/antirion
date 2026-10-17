//! Model metadata and list prices from models.dev (<https://models.dev>, MIT licence): each model's
//! name, limits, modalities, release date and per-token prices, for the provider types the gateway
//! implements. A trimmed snapshot is bundled; `cargo xtask catalog` refreshes it, and the optional
//! `catalog.remote` setting replaces it at run time with a fresh copy.
//!
//! The data is the lowest layer of the catalog: the bundled prices and the document's `catalog`
//! block come first wherever they define a model.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::{Arc, LazyLock, RwLock};

use serde_json::{Map, Value, json};

/// Where the data is published.
pub const SOURCE_URL: &str = "https://models.dev/api.json";

/// The bundled snapshot.
const BUNDLED_SNAPSHOT: &str = include_str!("../catalog/models-dev.json");

/// The models.dev providers each provider type reads, in lookup order: a type may read several
/// (Vertex serves Google's and Anthropic's models).
pub const PROVIDERS: &[(&str, &str)] = &[
    ("openai", "openai"),
    ("azure", "azure_openai"),
    ("anthropic", "anthropic"),
    ("amazon-bedrock", "bedrock"),
    ("google-vertex", "vertex"),
    ("google-vertex-anthropic", "vertex"),
    ("google", "gemini"),
    ("mistral", "mistral"),
    ("cohere", "cohere"),
    ("groq", "groq"),
    ("togetherai", "together"),
    ("fireworks-ai", "fireworks"),
    ("deepseek", "deepseek"),
    ("xai", "xai"),
    ("cerebras", "cerebras"),
    ("deepinfra", "deepinfra"),
    ("perplexity", "perplexity"),
];

/// Prices in USD per million tokens, as decimal strings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelPrice {
    pub input: String,
    pub output: String,
    pub cache_read: Option<String>,
    pub cache_write: Option<String>,
}

impl ModelPrice {
    /// The price in the catalog's price shape (`cache_write` is the five-minute write).
    #[must_use]
    pub fn to_catalog_price(&self) -> Value {
        let mut m = Map::new();
        m.insert("input".into(), Value::from(self.input.as_str()));
        m.insert("output".into(), Value::from(self.output.as_str()));
        if let Some(v) = &self.cache_read {
            m.insert("cache_read".into(), Value::from(v.as_str()));
        }
        if let Some(v) = &self.cache_write {
            m.insert("cache_write_5m".into(), Value::from(v.as_str()));
        }
        Value::Object(m)
    }

    /// A price of the catalog's price shape, when it has an input price.
    #[must_use]
    pub fn from_catalog_price(v: &Value) -> Option<ModelPrice> {
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
        Some(ModelPrice {
            input: text("input")?,
            output: text("output").unwrap_or_else(|| "0".to_owned()),
            cache_read: text("cache_read"),
            cache_write: text("cache_write_5m").or_else(|| text("cache_write")),
        })
    }

    fn to_value(&self) -> Value {
        json!({
            "input": self.input,
            "output": self.output,
            "cache_read": self.cache_read,
            "cache_write": self.cache_write,
        })
    }
}

/// What a model takes in and gives out (`text`, `image`, `audio`, `video`, `pdf`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Modalities {
    pub input: Vec<String>,
    pub output: Vec<String>,
}

/// One model's metadata.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: String,
    pub name: Option<String>,
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub modalities: Option<Modalities>,
    pub price: Option<ModelPrice>,
    pub release_date: Option<String>,
}

impl ModelInfo {
    /// The entry as the snapshot stores it (without its id, which is its key).
    fn to_snapshot_value(&self) -> Value {
        let mut m = Map::new();
        if let Some(n) = &self.name {
            m.insert("name".into(), Value::from(n.as_str()));
        }
        if let Some(p) = &self.price {
            let mut cost = Map::new();
            cost.insert("input".into(), Value::from(p.input.as_str()));
            cost.insert("output".into(), Value::from(p.output.as_str()));
            if let Some(v) = &p.cache_read {
                cost.insert("cache_read".into(), Value::from(v.as_str()));
            }
            if let Some(v) = &p.cache_write {
                cost.insert("cache_write".into(), Value::from(v.as_str()));
            }
            m.insert("cost".into(), Value::Object(cost));
        }
        if self.context_window.is_some() || self.max_output_tokens.is_some() {
            let mut limit = Map::new();
            if let Some(c) = self.context_window {
                limit.insert("context".into(), Value::from(c));
            }
            if let Some(o) = self.max_output_tokens {
                limit.insert("output".into(), Value::from(o));
            }
            m.insert("limit".into(), Value::Object(limit));
        }
        if let Some(md) = &self.modalities {
            m.insert(
                "modalities".into(),
                json!({"input": md.input, "output": md.output}),
            );
        }
        if let Some(d) = &self.release_date {
            m.insert("release_date".into(), Value::from(d.as_str()));
        }
        Value::Object(m)
    }

    /// Reads one model of models.dev's shape (its API or the snapshot: the same field names).
    fn from_models_dev(id: &str, v: &Value) -> Option<ModelInfo> {
        let v = v.as_object()?;
        let text = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let positive = |path: &str| {
            v.get("limit")
                .and_then(|l| l.get(path))
                .and_then(Value::as_u64)
                .filter(|n| *n > 0)
        };
        let list = |side: &str| -> Vec<String> {
            v.get("modalities")
                .and_then(|m| m.get(side))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        };
        let modalities = v.get("modalities").filter(|m| m.is_object()).map(|_| Modalities {
            input: list("input"),
            output: list("output"),
        });
        let cost = v.get("cost");
        let rate = |k: &str| cost.and_then(|c| c.get(k)).and_then(decimal);
        let price = rate("input").map(|input| ModelPrice {
            input,
            output: rate("output").unwrap_or_else(|| "0".to_owned()),
            cache_read: rate("cache_read"),
            cache_write: rate("cache_write"),
        });
        Some(ModelInfo {
            id: id.to_owned(),
            name: text("name"),
            context_window: positive("context"),
            max_output_tokens: positive("output"),
            modalities,
            price,
            release_date: text("release_date"),
        })
    }

    /// The model as the Admin API shows it (`ProviderModel` without `listed_by_provider`).
    #[must_use]
    pub fn to_value(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "context_window": self.context_window,
            "max_output_tokens": self.max_output_tokens,
            "modalities": self.modalities.as_ref().map(|m| json!({"input": m.input, "output": m.output})),
            "price": self.price.as_ref().map(ModelPrice::to_value),
            "price_source": self.price.as_ref().map(|_| "catalog"),
            "release_date": self.release_date,
        })
    }
}

/// A price as a decimal string: a JSON number (models.dev's form) or a decimal string (the
/// snapshot's); negative and non-finite values are refused.
fn decimal(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => {
            let valid = !s.is_empty()
                && s.chars().all(|c| c.is_ascii_digit() || c == '.')
                && s.chars().filter(|c| *c == '.').count() <= 1;
            valid.then(|| s.clone())
        }
        Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                return Some(u.to_string());
            }
            let f = n.as_f64().filter(|f| f.is_finite() && *f >= 0.0)?;
            // `Display` writes the shortest form that reads back as the same value, never with
            // an exponent.
            Some(format!("{f}"))
        }
        _ => None,
    }
}

/// The data of one models.dev copy, by provider type and model id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelsDev {
    /// When the copy was taken, `YYYY-MM-DD`.
    pub date: String,
    types: BTreeMap<String, BTreeMap<String, ModelInfo>>,
}

/// Why a copy could not be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ReadError(pub String);

impl ModelsDev {
    /// Trims models.dev's whole API document to the provider types the gateway implements.
    ///
    /// # Errors
    /// When the document is not an object of providers or has none of the known ones.
    pub fn from_api(api: &Value, date: &str) -> Result<ModelsDev, ReadError> {
        let root = api
            .as_object()
            .ok_or_else(|| ReadError("the document is not an object".to_owned()))?;
        let mut types: BTreeMap<String, BTreeMap<String, ModelInfo>> = BTreeMap::new();
        for (source, kind) in PROVIDERS {
            let Some(models) = root
                .get(*source)
                .and_then(|p| p.get("models"))
                .and_then(Value::as_object)
            else {
                continue;
            };
            let into = types.entry((*kind).to_owned()).or_default();
            for (id, m) in models {
                let id = m.get("id").and_then(Value::as_str).unwrap_or(id);
                if let Some(info) = ModelInfo::from_models_dev(id, m) {
                    into.entry(id.to_owned()).or_insert(info);
                }
            }
        }
        if types.is_empty() {
            return Err(ReadError(
                "the document has none of the known providers".to_owned(),
            ));
        }
        Ok(ModelsDev {
            date: date.to_owned(),
            types,
        })
    }

    /// Reads a snapshot written by [`ModelsDev::to_snapshot`].
    ///
    /// # Errors
    /// When the text is not a snapshot.
    pub fn from_snapshot(text: &str) -> Result<ModelsDev, ReadError> {
        let v: Value = serde_json::from_str(text).map_err(|e| ReadError(e.to_string()))?;
        let types = v
            .get("types")
            .and_then(Value::as_object)
            .ok_or_else(|| ReadError("the snapshot has no types".to_owned()))?
            .iter()
            .map(|(kind, models)| {
                let models = models
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter_map(|(id, m)| Some((id.clone(), ModelInfo::from_models_dev(id, m)?)))
                    .collect();
                (kind.clone(), models)
            })
            .collect();
        Ok(ModelsDev {
            date: v
                .get("date")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            types,
        })
    }

    /// The snapshot text: a header, then one line per model, ordered by type and id, so a refresh
    /// reads as a short diff.
    #[must_use]
    pub fn to_snapshot(&self) -> String {
        let mut out = String::new();
        out.push_str("{\n");
        let _ = writeln!(out, "  \"source\": {},", Value::from(SOURCE_URL));
        out.push_str("  \"license\": \"MIT; Copyright (c) 2025 models.dev; see THIRD-PARTY-NOTICES.md next to this file\",\n");
        let _ = writeln!(out, "  \"date\": {},", Value::from(self.date.as_str()));
        out.push_str("  \"types\": {");
        let mut first_type = true;
        for (kind, models) in &self.types {
            out.push_str(if first_type { "\n" } else { ",\n" });
            first_type = false;
            let _ = write!(out, "    {}: {{", Value::from(kind.as_str()));
            let mut first = true;
            for (id, m) in models {
                out.push_str(if first { "\n" } else { ",\n" });
                first = false;
                let _ = write!(
                    out,
                    "      {}: {}",
                    Value::from(id.as_str()),
                    m.to_snapshot_value()
                );
            }
            out.push_str("\n    }");
        }
        out.push_str("\n  }\n}\n");
        out
    }

    /// The model `id` of a `provider_type` provider.
    #[must_use]
    pub fn get(&self, provider_type: &str, id: &str) -> Option<&ModelInfo> {
        self.types.get(provider_type)?.get(id)
    }

    /// The model `id` under any type, in [`PROVIDERS`] order.
    #[must_use]
    pub fn get_any(&self, id: &str) -> Option<&ModelInfo> {
        let mut seen: Vec<&str> = Vec::new();
        for (_, kind) in PROVIDERS {
            if seen.contains(kind) {
                continue;
            }
            seen.push(kind);
            if let Some(m) = self.get(kind, id) {
                return Some(m);
            }
        }
        None
    }

    /// Every model of a provider type, by id.
    pub fn models(&self, provider_type: &str) -> impl Iterator<Item = &ModelInfo> {
        self.types
            .get(provider_type)
            .into_iter()
            .flat_map(BTreeMap::values)
    }

    /// The number of models over every type.
    #[must_use]
    pub fn len(&self) -> usize {
        self.types.values().map(BTreeMap::len).sum()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

static BUNDLED: LazyLock<Arc<ModelsDev>> =
    LazyLock::new(|| Arc::new(ModelsDev::from_snapshot(BUNDLED_SNAPSHOT).unwrap_or_default()));

/// A copy fetched at run time, which replaces the bundled one while it is set.
static REMOTE: RwLock<Option<Arc<ModelsDev>>> = RwLock::new(None);

/// The bundled snapshot.
#[must_use]
pub fn bundled() -> Arc<ModelsDev> {
    Arc::clone(&BUNDLED)
}

/// The copy in use: the last one fetched at run time, else the bundled snapshot.
#[must_use]
pub fn current() -> Arc<ModelsDev> {
    REMOTE.read().ok().and_then(|r| r.clone()).unwrap_or_else(bundled)
}

/// Puts a copy fetched at run time in place of the bundled one (`None` goes back to it). Catalogs
/// built afterwards read it.
pub fn install(copy: Option<ModelsDev>) {
    if let Ok(mut r) = REMOTE.write() {
        *r = copy.map(Arc::new);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api() -> Value {
        json!({
            "openai": {"id": "openai", "models": {
                "gpt-x": {"id": "gpt-x", "name": "GPT X", "release_date": "2026-01-02",
                          "modalities": {"input": ["text", "image"], "output": ["text"]},
                          "limit": {"context": 400000, "output": 128000},
                          "cost": {"input": 1.25, "output": 10, "cache_read": 0.125,
                                   "context_over_200k": {"input": 2.5}}},
                "img": {"id": "img", "name": "Image", "limit": {"context": 0, "output": 0},
                        "cost": {"input": 0.00001, "output": 40}}
            }},
            "google-vertex": {"models": {"gemini-x": {"id": "gemini-x", "cost": {"input": 0.3, "output": 2.5}}}},
            "google-vertex-anthropic": {"models": {"claude-x@1": {"id": "claude-x@1",
                "cost": {"input": 3, "output": 15, "cache_read": 0.3, "cache_write": 3.75}}}},
            "someone-else": {"models": {"other": {"id": "other"}}}
        })
    }

    #[test]
    fn the_api_document_is_trimmed_to_the_known_types() {
        let m = ModelsDev::from_api(&api(), "2026-10-17").unwrap();
        assert_eq!(m.len(), 4);
        let gpt = m.get("openai", "gpt-x").unwrap();
        assert_eq!(gpt.name.as_deref(), Some("GPT X"));
        assert_eq!(
            (gpt.context_window, gpt.max_output_tokens),
            (Some(400_000), Some(128_000))
        );
        let p = gpt.price.as_ref().unwrap();
        assert_eq!(
            (p.input.as_str(), p.output.as_str(), p.cache_read.as_deref()),
            ("1.25", "10", Some("0.125"))
        );
        assert_eq!(
            m.get("openai", "img").unwrap().price.as_ref().unwrap().input,
            "0.00001"
        );
        assert_eq!(m.get("openai", "img").unwrap().context_window, None);
        assert!(m.get("vertex", "gemini-x").is_some() && m.get("vertex", "claude-x@1").is_some());
        assert!(m.get_any("other").is_none());
        assert_eq!(m.get_any("claude-x@1").unwrap().id, "claude-x@1");
        assert!(ModelsDev::from_api(&json!({"x": {}}), "d").is_err());
    }

    #[test]
    fn a_snapshot_round_trips() {
        let m = ModelsDev::from_api(&api(), "2026-10-17").unwrap();
        let text = m.to_snapshot();
        assert_eq!(ModelsDev::from_snapshot(&text).unwrap(), m);
        assert!(text.lines().any(|l| l.trim_start().starts_with("\"gpt-x\": {")));
    }

    #[test]
    fn the_bundled_snapshot_reads() {
        let b = bundled();
        assert!(b.len() > 100, "{} models", b.len());
        assert_eq!(b.date.len(), 10);
        for kind in [
            "openai",
            "anthropic",
            "gemini",
            "bedrock",
            "vertex",
            "mistral",
            "cohere",
        ] {
            assert!(b.models(kind).next().is_some(), "no {kind} model");
        }
    }

    #[test]
    fn catalog_prices_convert_both_ways() {
        let p = ModelPrice {
            input: "3".into(),
            output: "15".into(),
            cache_read: Some("0.3".into()),
            cache_write: Some("3.75".into()),
        };
        let v = p.to_catalog_price();
        assert_eq!(v["cache_write_5m"], json!("3.75"));
        assert_eq!(ModelPrice::from_catalog_price(&v), Some(p));
        assert_eq!(
            ModelPrice::from_catalog_price(&json!({"input": "0.02"}))
                .unwrap()
                .output,
            "0"
        );
        assert!(decimal(&json!(-1.0)).is_none());
        assert_eq!(decimal(&json!("1.5")), Some("1.5".to_owned()));
        assert!(decimal(&json!("1e5")).is_none());
    }
}
