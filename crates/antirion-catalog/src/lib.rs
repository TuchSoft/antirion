//! The model catalog: capability values, time-to-first-token hints and prices per model pattern,
//! from the bundled files and the document's `catalog` block, which extends or replaces them. Below
//! both, models.dev's data ([`modelsdev`]) gives the prices of the models they do not price, and the
//! metadata (name, limits, modalities) of each provider type's models.

#![forbid(unsafe_code)]

pub mod modelsdev;

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use serde_json::{Map, Value};

use crate::modelsdev::{ModelInfo, ModelPrice, ModelsDev};

/// The bundled prices, `{version, models: {pattern: price}}`.
pub const BUNDLED_PRICES: &str = include_str!("../catalog/prices.json");

/// The bundled capability values and hints, `{version, models: {pattern: entry}}`.
const BUNDLED_MODELS: &str = include_str!("../catalog/models.json");

/// One pattern's data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    /// Capability flags the entry sets.
    pub capabilities: Map<String, Value>,
    /// The time-to-first-token hint.
    pub ttft: Option<Duration>,
    /// The price, as written (`Price` of the limits engine reads it).
    pub price: Option<Value>,
}

impl Entry {
    fn from_value(v: &Value) -> Entry {
        Entry {
            capabilities: v
                .get("capabilities")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
            ttft: v
                .get("ttft")
                .and_then(Value::as_str)
                .and_then(antirion_config::scalar::parse_duration),
            price: v.get("price").filter(|p| p.is_object()).cloned(),
        }
    }

    /// The entry as the catalog read shows it.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let mut m = Map::new();
        m.insert("capabilities".into(), Value::Object(self.capabilities.clone()));
        m.insert(
            "ttft".into(),
            self.ttft.map_or(Value::Null, |d| {
                Value::from(antirion_config::scalar::format_duration(d))
            }),
        );
        m.insert("price".into(), self.price.clone().unwrap_or(Value::Null));
        Value::Object(m)
    }
}

/// Where an entry of the effective catalog comes from.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Bundled,
    Runtime,
}

impl Source {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Bundled => "bundled",
            Source::Runtime => "runtime",
        }
    }
}

/// How the document's catalog combines with the bundled one.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    Extend,
    Replace,
}

impl Mode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Extend => "extend",
            Mode::Replace => "replace",
        }
    }
}

/// What the document's `catalog` block says about itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Runtime {
    pub mode: Mode,
    pub version: Option<String>,
    pub entries: usize,
}

/// What the bundled catalog says about itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bundled {
    /// The version of the capability file.
    pub version: String,
    /// The version of the bundled prices.
    pub price_version: String,
    pub entries: usize,
}

/// The effective catalog of one configuration generation.
#[derive(Clone, Debug)]
pub struct Catalog {
    entries: BTreeMap<String, (Arc<Entry>, Source)>,
    /// Patterns ending in `*`, without it, longest first.
    prefixes: Vec<String>,
    pub bundled: Bundled,
    pub runtime: Option<Runtime>,
    /// models.dev's data, the lowest layer; none when the document's catalog replaces the bundled
    /// one.
    extra: Option<Arc<ModelsDev>>,
}

fn version_of(v: &Value) -> String {
    v.get("version")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// The bundled entries: the capability file's, with the bundled prices merged in by pattern.
fn bundled_entries() -> (BTreeMap<String, Entry>, Bundled) {
    let models: Value = serde_json::from_str(BUNDLED_MODELS).unwrap_or(Value::Null);
    let prices: Value = serde_json::from_str(BUNDLED_PRICES).unwrap_or(Value::Null);
    let mut out: BTreeMap<String, Entry> = models
        .get("models")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(k, v)| (k.clone(), Entry::from_value(v)))
        .collect();
    for (k, p) in prices
        .get("models")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        out.entry(k.clone()).or_default().price = Some(p.clone());
    }
    let info = Bundled {
        version: version_of(&models),
        price_version: version_of(&prices),
        entries: out.len(),
    };
    (out, info)
}

static BUNDLED: LazyLock<(BTreeMap<String, Entry>, Bundled)> = LazyLock::new(bundled_entries);

impl Catalog {
    /// The bundled catalog alone.
    #[must_use]
    pub fn bundled() -> Catalog {
        Catalog::from_block(None)
    }

    /// The effective catalog of a resolved document.
    #[must_use]
    pub fn from_doc(doc: &Value) -> Catalog {
        Catalog::from_block(doc.get("catalog").filter(|c| c.is_object()))
    }

    fn from_block(block: Option<&Value>) -> Catalog {
        let (bundled, info) = &*BUNDLED;
        let mode = match block.and_then(|b| b.get("mode")).and_then(Value::as_str) {
            Some("replace") => Mode::Replace,
            _ => Mode::Extend,
        };
        let mut entries: BTreeMap<String, (Arc<Entry>, Source)> = if mode == Mode::Replace {
            BTreeMap::new()
        } else {
            bundled
                .iter()
                .map(|(k, e)| (k.clone(), (Arc::new(e.clone()), Source::Bundled)))
                .collect()
        };
        let own = block.and_then(|b| b.get("models")).and_then(Value::as_object);
        for (k, v) in own.into_iter().flatten() {
            entries.insert(k.clone(), (Arc::new(Entry::from_value(v)), Source::Runtime));
        }
        let mut prefixes: Vec<String> = entries
            .keys()
            .filter_map(|k| k.strip_suffix('*').map(str::to_owned))
            .collect();
        prefixes.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        Catalog {
            entries,
            prefixes,
            bundled: info.clone(),
            runtime: block.map(|b| Runtime {
                mode,
                version: b.get("version").and_then(Value::as_str).map(str::to_owned),
                entries: own.map_or(0, Map::len),
            }),
            extra: (mode == Mode::Extend).then(modelsdev::current),
        }
    }

    /// The same catalog over another copy of models.dev's data.
    #[must_use]
    pub fn with_models_dev(mut self, data: Option<Arc<ModelsDev>>) -> Catalog {
        if self.extra.is_some() {
            self.extra = data;
        }
        self
    }

    /// The date of the models.dev data the catalog reads, when it reads any.
    #[must_use]
    pub fn models_dev_date(&self) -> Option<&str> {
        self.extra.as_deref().map(|m| m.date.as_str())
    }

    /// The price of `base_model` on a `provider_type` provider: the price of the entry
    /// [`Catalog::lookup`] finds, else models.dev's list price for `<type>/<model>`.
    #[must_use]
    pub fn price_of(&self, provider_type: &str, base_model: &str) -> Option<Value> {
        if let Some(p) = self
            .lookup(provider_type, base_model)
            .and_then(|(e, _)| e.price.clone())
        {
            return Some(p);
        }
        self.extra
            .as_deref()?
            .get(provider_type, base_model)?
            .price
            .as_ref()
            .map(ModelPrice::to_catalog_price)
    }

    /// What the catalog knows of the model `id` of a `provider_type` provider: models.dev's
    /// metadata (for a generic type, the same id under any type), with the price of an entry of the
    /// bundled prices or the document in place of models.dev's. `None` when nothing knows it.
    #[must_use]
    pub fn model_info(&self, provider_type: &str, id: &str, generic: bool) -> Option<ModelInfo> {
        let extra = self.extra.as_deref().and_then(|m| {
            m.get(provider_type, id)
                .or_else(|| generic.then(|| m.get_any(id)).flatten())
        });
        let entry = if generic {
            self.lookup_any_type(provider_type, id)
        } else {
            self.lookup(provider_type, id)
        }
        .map(|(e, _)| e);
        let own_price = entry
            .and_then(|e| e.price.as_ref())
            .and_then(ModelPrice::from_catalog_price);
        let mut info = match (extra, entry) {
            (Some(m), _) => m.clone(),
            (None, Some(e)) => ModelInfo {
                id: id.to_owned(),
                context_window: e.capabilities.get("context_window").and_then(Value::as_u64),
                max_output_tokens: e.capabilities.get("max_output_tokens").and_then(Value::as_u64),
                ..ModelInfo::default()
            },
            (None, None) => return None,
        };
        info.id = id.to_owned();
        if own_price.is_some() {
            info.price = own_price;
        }
        Some(info)
    }

    /// The models the catalog knows for a `provider_type` provider, by id: models.dev's for the
    /// type and the entries named `<type>/<model>` (without a `*`), each as
    /// [`Catalog::model_info`] reads it.
    #[must_use]
    pub fn models_of_type(&self, provider_type: &str) -> Vec<ModelInfo> {
        let mut ids: std::collections::BTreeSet<String> = self
            .extra
            .as_deref()
            .into_iter()
            .flat_map(|m| m.models(provider_type))
            .map(|m| m.id.clone())
            .collect();
        let prefix = format!("{provider_type}/");
        ids.extend(
            self.entries
                .keys()
                .filter_map(|k| k.strip_prefix(&prefix))
                .filter(|m| !m.ends_with('*'))
                .map(str::to_owned),
        );
        ids.iter()
            .filter_map(|id| self.model_info(provider_type, id, false))
            .collect()
    }

    /// The entry of a deployment of a `provider_type` provider serving `base_model`: the exact
    /// `<type>/<model>` pattern, then the exact `<model>`, then the longest `*` prefix (a
    /// type-qualified prefix before a bare one of the same length).
    #[must_use]
    pub fn lookup(&self, provider_type: &str, base_model: &str) -> Option<(&Entry, Source)> {
        self.find(provider_type, base_model, false)
    }

    /// [`Catalog::lookup`] for a provider of a generic type, which serves other vendors' models
    /// under their own ids: when its own type has no entry, an entry another type qualifies for
    /// the same model id counts as a bare one, after the bare ones (the first by pattern when
    /// several types have one).
    #[must_use]
    pub fn lookup_any_type(&self, provider_type: &str, base_model: &str) -> Option<(&Entry, Source)> {
        self.find(provider_type, base_model, true)
    }

    fn find(&self, provider_type: &str, base_model: &str, any_type: bool) -> Option<(&Entry, Source)> {
        let qualified = format!("{provider_type}/{base_model}");
        let other_type = || {
            any_type
                .then(|| {
                    self.entries
                        .iter()
                        .find(|(k, _)| k.split_once('/').is_some_and(|(_, m)| m == base_model))
                        .map(|(_, v)| v)
                })
                .flatten()
        };
        if let Some((e, s)) = self
            .entries
            .get(&qualified)
            .or_else(|| self.entries.get(base_model))
            .or_else(other_type)
        {
            return Some((e.as_ref(), *s));
        }
        let mut best: Option<(usize, u8, &str)> = None;
        for p in &self.prefixes {
            let split = p.split_once('/');
            let rank = match split {
                Some((t, m)) if t == provider_type && base_model.starts_with(m) => Some((m.len(), 2)),
                _ if base_model.starts_with(p.as_str()) => Some((p.len(), 1)),
                Some((_, m)) if any_type && base_model.starts_with(m) => Some((m.len(), 0)),
                _ => None,
            };
            let Some((matched, rank)) = rank else {
                continue;
            };
            if best.is_none_or(|(len, was, _)| matched > len || (matched == len && rank > was)) {
                best = Some((matched, rank, p.as_str()));
            }
        }
        let (_, _, p) = best?;
        self.entries.get(&format!("{p}*")).map(|(e, s)| (e.as_ref(), *s))
    }

    /// Every entry, by pattern.
    pub fn items(&self) -> impl Iterator<Item = (&str, &Entry, Source)> {
        self.entries
            .iter()
            .map(|(k, (e, s))| (k.as_str(), e.as_ref(), *s))
    }

    /// The prices of the effective catalog, by pattern.
    pub fn prices(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.entries
            .iter()
            .filter_map(|(k, (e, _))| e.price.as_ref().map(|p| (k.as_str(), p)))
    }
}

impl Default for Catalog {
    fn default() -> Catalog {
        Catalog::bundled()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_bundled_catalog_carries_prices() {
        let c = Catalog::bundled();
        assert!(!c.bundled.price_version.is_empty());
        let (e, s) = c.lookup("openai", "gpt-4o").unwrap();
        assert_eq!(s, Source::Bundled);
        assert!(e.price.is_some());
        assert!(c.runtime.is_none());
    }

    #[test]
    fn runtime_entries_extend_or_replace() {
        let doc = json!({"catalog": {"version": "v2", "models": {
            "openai/gpt-4o": {"capabilities": {"max_output_tokens": 100}, "ttft": "90s"},
            "acme-*": {"capabilities": {"tools": false}},
            "acme-large-*": {"capabilities": {"tools": true}},
            "openai/acme-*": {"capabilities": {"vision": false}},
        }}});
        let c = Catalog::from_doc(&doc);
        let (e, s) = c.lookup("openai", "gpt-4o").unwrap();
        assert_eq!(s, Source::Runtime);
        assert_eq!(e.ttft, Some(Duration::from_secs(90)));
        assert!(
            e.price.is_none(),
            "a runtime entry replaces the bundled one whole"
        );
        assert!(
            c.lookup("openai", "gpt-4o-mini")
                .is_some_and(|(_, s)| s == Source::Bundled)
        );
        let tools = |t: &str, m: &str| c.lookup(t, m).map(|(e, _)| e.capabilities.clone());
        assert_eq!(tools("anthropic", "acme-large-1").unwrap()["tools"], json!(true));
        assert_eq!(tools("anthropic", "acme-small").unwrap()["tools"], json!(false));
        assert_eq!(tools("openai", "acme-small").unwrap()["vision"], json!(false));
        assert!(c.lookup("anthropic", "other").is_none());
        let rt = c.runtime.unwrap();
        assert_eq!(
            (rt.mode, rt.version.as_deref(), rt.entries),
            (Mode::Extend, Some("v2"), 4)
        );

        let replaced = Catalog::from_doc(&json!({"catalog": {"mode": "replace", "models": {"m": {}}}}));
        assert!(replaced.lookup("openai", "gpt-4o-mini").is_none());
        assert!(replaced.lookup("openai", "m").is_some());
        assert_eq!(replaced.prices().count(), 0);
    }

    #[test]
    fn a_generic_type_reads_the_entry_of_its_model_id_under_any_type() {
        let doc = json!({"catalog": {"models": {
            "openai/gpt-4o": {"capabilities": {"context_window": 128_000}},
            "anthropic/claude-x*": {"capabilities": {"context_window": 200_000}},
            "llama-3": {"capabilities": {"context_window": 8_192}},
            "openai/llama-3": {"capabilities": {"context_window": 1}},
            "openai_compatible/own": {"capabilities": {"context_window": 2}},
            "openai/own": {"capabilities": {"context_window": 3}},
        }}});
        let c = Catalog::from_doc(&doc);
        let window = |any: bool, m: &str| {
            let found = if any {
                c.lookup_any_type("openai_compatible", m)
            } else {
                c.lookup("openai_compatible", m)
            };
            found.map(|(e, _)| e.capabilities["context_window"].clone())
        };
        assert_eq!(
            window(false, "gpt-4o"),
            None,
            "a typed entry of another type is not the type's own"
        );
        assert_eq!(window(true, "gpt-4o"), Some(json!(128_000)));
        assert_eq!(window(true, "claude-x-1"), Some(json!(200_000)));
        assert_eq!(
            window(true, "llama-3"),
            Some(json!(8_192)),
            "a bare entry before another type's"
        );
        assert_eq!(window(true, "own"), Some(json!(2)), "the type's own entry first");
        assert_eq!(window(true, "unknown"), None);
    }

    fn models_dev() -> Option<Arc<ModelsDev>> {
        let api = json!({
            "openai": {"models": {
                "gpt-4o": {"id": "gpt-4o", "name": "GPT-4o", "limit": {"context": 128000, "output": 16384},
                           "cost": {"input": 9, "output": 9}},
                "gpt-4o-mini": {"id": "gpt-4o-mini", "name": "GPT-4o mini", "cost": {"input": 0.15, "output": 0.6}},
                "gpt-new": {"id": "gpt-new", "name": "GPT New", "cost": {"input": 1, "output": 2, "cache_read": 0.1}},
                "acme-1": {"id": "acme-1", "cost": {"input": 7, "output": 7}}
            }},
            "anthropic": {"models": {"claude-z": {"id": "claude-z", "name": "Claude Z",
                "modalities": {"input": ["text", "image"], "output": ["text"]},
                "cost": {"input": 3, "output": 15, "cache_read": 0.3, "cache_write": 3.75}}}}
        });
        Some(Arc::new(ModelsDev::from_api(&api, "2026-10-17").unwrap()))
    }

    fn input_price(c: &Catalog, t: &str, m: &str) -> Option<String> {
        c.price_of(t, m)
            .and_then(|p| p.get("input").and_then(Value::as_str).map(str::to_owned))
    }

    #[test]
    fn models_dev_fills_in_what_the_bundled_prices_and_the_document_do_not_price() {
        let doc = json!({"catalog": {"models": {
            "openai/gpt-4o-mini": {"price": {"input": "0.99", "output": "1.99"}},
            "openai/acme-*": {"price": {"input": "5.00", "output": "5.00"}},
            "openai/caps-only": {"capabilities": {"context_window": 4096}},
        }}});
        let c = Catalog::from_doc(&doc).with_models_dev(models_dev());
        assert_eq!(c.models_dev_date(), Some("2026-10-17"));
        assert_eq!(
            input_price(&c, "openai", "gpt-4o").as_deref(),
            Some("2.50"),
            "the bundled price before models.dev's"
        );
        assert_eq!(
            input_price(&c, "openai", "gpt-4o-mini").as_deref(),
            Some("0.99"),
            "the document's entry first"
        );
        assert_eq!(
            input_price(&c, "openai", "acme-1").as_deref(),
            Some("5.00"),
            "a document prefix before a models.dev exact id"
        );
        assert_eq!(input_price(&c, "openai", "gpt-new").as_deref(), Some("1"));
        let claude = c.price_of("anthropic", "claude-z").unwrap();
        assert_eq!(claude["cache_write_5m"], json!("3.75"));
        assert!(c.price_of("openai", "unknown").is_none());
        assert!(
            c.price_of("anthropic", "gpt-new").is_none(),
            "prices keep the type's own lookup"
        );

        let info = c.model_info("openai", "gpt-4o", false).unwrap();
        assert_eq!(
            (info.name.as_deref(), info.context_window),
            (Some("GPT-4o"), Some(128_000))
        );
        assert_eq!(
            info.price.unwrap().input,
            "2.50",
            "metadata from models.dev, the price from the bundled prices"
        );
        let caps = c.model_info("openai", "caps-only", false).unwrap();
        assert_eq!((caps.context_window, caps.price), (Some(4096), None));
        assert!(c.model_info("openai", "nothing", false).is_none());
        assert!(c.model_info("openai_compatible", "claude-z", false).is_none());
        assert_eq!(
            c.model_info("openai_compatible", "claude-z", true)
                .unwrap()
                .name
                .as_deref(),
            Some("Claude Z"),
            "a generic type reads the id under any type"
        );

        let ids: Vec<String> = c.models_of_type("openai").into_iter().map(|m| m.id).collect();
        assert_eq!(
            ids,
            [
                "acme-1",
                "caps-only",
                "gpt-4.1",
                "gpt-4.1-mini",
                "gpt-4.1-nano",
                "gpt-4o",
                "gpt-4o-mini",
                "gpt-5",
                "gpt-5-mini",
                "gpt-5-nano",
                "gpt-new",
                "o3",
                "o4-mini",
                "text-embedding-3-large",
                "text-embedding-3-small"
            ]
        );
    }

    #[test]
    fn a_replacing_document_drops_models_dev_too() {
        let c = Catalog::from_doc(&json!({"catalog": {"mode": "replace", "models": {}}}))
            .with_models_dev(models_dev());
        assert!(c.models_dev_date().is_none());
        assert!(c.price_of("openai", "gpt-new").is_none());
        assert!(c.models_of_type("openai").is_empty());
    }

    #[test]
    fn the_bundled_models_dev_snapshot_prices_models_the_bundled_prices_lack() {
        let c = Catalog::bundled();
        assert!(c.models_dev_date().is_some());
        assert!(c.price_of("anthropic", "claude-sonnet-4-5").is_some());
        assert!(c.models_of_type("gemini").len() > 5);
    }
}
