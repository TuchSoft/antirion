//! A provider's own model list: the listing call of its type (OpenAI's `GET /models` and the
//! shapes Azure, Mistral, Anthropic, Gemini, Cohere and Bedrock give it), its parsers, and what a
//! node remembers of listings and credential tests.
//!
//! The request is built like any upstream request of the provider (base URL, required and
//! configured headers, the chosen key, TLS profile and egress policy) and authorized by
//! [`DeploymentRt::authorize`] on a deployment made for the purpose ([`DeploymentRt::direct`]).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use antirion_core::{RequestId, UtcMicros, headers as h};
use antirion_egress::{OutboundRequest, Payload, TrustClass};
use http::{HeaderMap, HeaderName, HeaderValue, Method, Uri};
use parking_lot::Mutex;
use serde_json::Value;

use crate::url::join;
use crate::{BuildError, DeploymentRt, ProviderRt, ProviderType, UpstreamAuth};

/// The listing call of a provider's type.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Listing {
    /// `GET {base}/models`, OpenAI's shape (OpenAI, OpenAI-compatible servers, the presets).
    OpenAi,
    /// Azure OpenAI's models of the resource: `/openai/models?api-version=…`, or
    /// `/openai/v1/models` for the v1 API.
    Azure,
    /// Mistral's `GET /v1/models`, whose entries carry capabilities.
    Mistral,
    /// `GET /v1/models`, paginated by `after_id`.
    Anthropic,
    /// `models.list`, paginated by `pageToken`.
    Gemini,
    /// Cohere's `GET /v1/models`, paginated by `page_token`.
    Cohere,
    /// Bedrock's `ListFoundationModels`, on the control-plane host of the provider's region.
    Bedrock,
}

impl ProviderRt {
    /// The listing call of the provider, when its type has one the gateway can make: Vertex has
    /// none, nor has a Bedrock provider behind a base URL of its own (a private endpoint of the
    /// runtime API, which does not serve the control plane).
    #[must_use]
    pub fn listing(&self) -> Option<Listing> {
        Some(match self.kind {
            ProviderType::AzureOpenAi => Listing::Azure,
            ProviderType::Mistral => Listing::Mistral,
            k if k.openai_family() => Listing::OpenAi,
            k if k.anthropic_api() => Listing::Anthropic,
            ProviderType::Gemini => Listing::Gemini,
            ProviderType::Cohere => Listing::Cohere,
            ProviderType::Bedrock if self.base_url.is_none() => Listing::Bedrock,
            _ => return None,
        })
    }
}

/// The page size asked for where the API takes one.
const PAGE_SIZE: &str = "1000";

impl DeploymentRt {
    /// The request of one page of the provider's model list (`page` is the token of the page
    /// after the first), with the key `key` (the provider's first when `None`). The credential is
    /// added by [`DeploymentRt::authorize`].
    ///
    /// # Errors
    /// When the provider's auth method is not supported or its URL is invalid.
    pub fn listing_request(
        &self,
        listing: Listing,
        key: Option<&str>,
        request_id: &RequestId,
        page: Option<&str>,
        timeout: Duration,
    ) -> Result<OutboundRequest, BuildError> {
        let p = &self.provider;
        if let UpstreamAuth::Unsupported(_) = p.auth {
            return Err(BuildError::Unsupported);
        }
        let own = p.operations.get("models").map(String::as_str);
        let mut query: Vec<(&str, &str)> = Vec::new();
        let (base, path) = match listing {
            Listing::OpenAi | Listing::Mistral => (self.base()?, own.unwrap_or("/models").to_owned()),
            Listing::Azure => match &p.azure_deployments {
                Some(version) => {
                    query.push(("api-version", version));
                    (self.base()?, own.unwrap_or("/openai/models").to_owned())
                }
                None => (self.base()?, own.unwrap_or("/openai/v1/models").to_owned()),
            },
            Listing::Anthropic => {
                query.push(("limit", PAGE_SIZE));
                if let Some(after) = page {
                    query.push(("after_id", after));
                }
                (self.base()?, "/v1/models".to_owned())
            }
            Listing::Gemini => {
                query.push(("pageSize", PAGE_SIZE));
                if let Some(token) = page {
                    query.push(("pageToken", token));
                }
                (self.base()?, format!("/{}/models", p.gemini_version))
            }
            Listing::Cohere => {
                query.push(("page_size", PAGE_SIZE));
                if let Some(token) = page {
                    query.push(("page_token", token));
                }
                (self.base()?, "/v1/models".to_owned())
            }
            Listing::Bedrock => {
                let region = self.region.as_deref().unwrap_or("us-east-1");
                (
                    format!("https://bedrock.{region}.amazonaws.com"),
                    "/foundation-models".to_owned(),
                )
            }
        };
        let uri: Uri = join(&base, &path, &query)
            .parse()
            .map_err(|_| BuildError::InvalidUrl)?;
        let mut headers = HeaderMap::new();
        if let Ok(v) = HeaderValue::from_str(request_id.as_str()) {
            if p.kind.openai_family() {
                headers.insert(HeaderName::from_static(h::X_CLIENT_REQUEST_ID), v.clone());
            }
            headers.insert(HeaderName::from_static(h::X_REQUEST_ID), v);
        }
        if listing == Listing::Anthropic
            && let Ok(v) = HeaderValue::from_str(&p.anthropic.version)
        {
            headers.insert(HeaderName::from_static("anthropic-version"), v);
        }
        for (name, value) in p.required.iter().chain(p.headers.iter()) {
            headers.insert(name.clone(), value.clone());
        }
        if let UpstreamAuth::Header { name, value } = &p.auth {
            let chosen = key
                .and_then(|k| p.keys.iter().find(|pk| pk.name == k))
                .map_or(value, |pk| &pk.value);
            headers.insert(name.clone(), chosen.clone());
        }
        Ok(OutboundRequest {
            method: Method::GET,
            uri,
            headers,
            body: Payload::from(bytes::Bytes::new()),
            response_timeout: timeout,
            connect_timeout: None,
            fresh_connection: false,
            trust: TrustClass::Operator,
            tls: p.tls.clone(),
            policy: Arc::clone(&p.egress),
            resend_when_closed: false,
        })
    }
}

/// One model of a provider's list, with what the provider said of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listed {
    pub id: String,
    pub name: Option<String>,
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
    /// Input and output modalities, lowercase, where the provider gives them.
    pub modalities: Option<(Vec<String>, Vec<String>)>,
}

/// One page of a list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub models: Vec<Listed>,
    /// The token of the next page, when there is one.
    pub next: Option<String>,
}

/// Why an answer is not a model list.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the provider's model list could not be read: {0}")]
pub struct ParseError(pub &'static str);

fn text(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn number(v: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|k| v.get(*k).and_then(Value::as_u64))
        .filter(|n| *n > 0)
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_ascii_lowercase)
        .collect()
}

fn array<'a>(v: &'a Value, key: &str) -> Result<&'a Vec<Value>, ParseError> {
    v.get(key)
        .and_then(Value::as_array)
        .ok_or(ParseError("no list of models in the answer"))
}

fn flag(v: &Value, key: &str) -> Option<bool> {
    v.get(key).and_then(Value::as_bool)
}

/// Entry types some OpenAI-compatible vendors mark that are not text generation or embeddings.
const NOT_TEXT_TYPES: &[&str] = &[
    "image",
    "audio",
    "video",
    "moderation",
    "transcribe",
    "transcription",
    "tts",
    "speech",
];

fn parse_openai(v: &Value) -> Result<Page, ParseError> {
    let models = array(v, "data")?
        .iter()
        .filter(|m| {
            m.get("type")
                .and_then(Value::as_str)
                .is_none_or(|t| !NOT_TEXT_TYPES.contains(&t.to_ascii_lowercase().as_str()))
        })
        .filter_map(|m| {
            Some(Listed {
                id: text(m, &["id"])?,
                name: text(m, &["display_name", "name"]),
                context_window: number(
                    m,
                    &[
                        "context_window",
                        "context_length",
                        "max_context_length",
                        "max_model_len",
                    ],
                ),
                max_output_tokens: number(m, &["max_output_tokens", "max_completion_tokens"]),
                modalities: None,
            })
        })
        .collect();
    Ok(Page { models, next: None })
}

fn parse_azure(v: &Value) -> Result<Page, ParseError> {
    let models = array(v, "data")?
        .iter()
        .filter(|m| match m.get("capabilities") {
            Some(c) if c.is_object() => ["chat_completion", "completion", "embeddings"]
                .iter()
                .any(|k| flag(c, k) == Some(true)),
            _ => true,
        })
        .filter_map(|m| {
            Some(Listed {
                id: text(m, &["id"])?,
                ..Listed::default()
            })
        })
        .collect();
    Ok(Page { models, next: None })
}

fn parse_mistral(v: &Value) -> Result<Page, ParseError> {
    let models = array(v, "data")?
        .iter()
        .filter(|m| {
            let Some(c) = m.get("capabilities").filter(|c| c.is_object()) else {
                return true;
            };
            let id = m.get("id").and_then(Value::as_str).unwrap_or_default();
            flag(c, "completion_chat") != Some(false)
                || flag(c, "completion_fim") == Some(true)
                || id.contains("embed")
        })
        .filter_map(|m| {
            Some(Listed {
                id: text(m, &["id"])?,
                name: text(m, &["name"]).filter(|n| Some(n.as_str()) != m.get("id").and_then(Value::as_str)),
                context_window: number(m, &["max_context_length"]),
                ..Listed::default()
            })
        })
        .collect();
    Ok(Page { models, next: None })
}

fn parse_anthropic(v: &Value) -> Result<Page, ParseError> {
    let models: Vec<Listed> = array(v, "data")?
        .iter()
        .filter_map(|m| {
            Some(Listed {
                id: text(m, &["id"])?,
                name: text(m, &["display_name"]),
                context_window: number(m, &["max_input_tokens"]),
                max_output_tokens: number(m, &["max_tokens"]),
                modalities: None,
            })
        })
        .collect();
    let next = (flag(v, "has_more") == Some(true))
        .then(|| text(v, &["last_id"]))
        .flatten();
    Ok(Page { models, next })
}

/// The Gemini methods of text generation and embeddings.
const GEMINI_METHODS: &[&str] = &[
    "generateContent",
    "streamGenerateContent",
    "embedContent",
    "batchEmbedContents",
];

fn parse_gemini(v: &Value) -> Result<Page, ParseError> {
    let models = v
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(
            |m| match m.get("supportedGenerationMethods").and_then(Value::as_array) {
                Some(methods) => methods
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|x| GEMINI_METHODS.contains(&x)),
                None => true,
            },
        )
        .filter_map(|m| {
            let full = text(m, &["name"])?;
            Some(Listed {
                id: full.strip_prefix("models/").unwrap_or(&full).to_owned(),
                name: text(m, &["displayName"]),
                context_window: number(m, &["inputTokenLimit"]),
                max_output_tokens: number(m, &["outputTokenLimit"]),
                modalities: None,
            })
        })
        .collect();
    if v.get("models").is_some_and(|m| !m.is_array()) || !v.is_object() {
        return Err(ParseError("no list of models in the answer"));
    }
    Ok(Page {
        models,
        next: text(v, &["nextPageToken"]),
    })
}

/// The Cohere endpoints of text generation, embeddings and reranking.
const COHERE_ENDPOINTS: &[&str] = &["chat", "generate", "embed", "rerank"];

fn parse_cohere(v: &Value) -> Result<Page, ParseError> {
    let models = array(v, "models")?
        .iter()
        .filter(|m| match m.get("endpoints").and_then(Value::as_array) {
            Some(e) => e
                .iter()
                .filter_map(Value::as_str)
                .any(|x| COHERE_ENDPOINTS.contains(&x)),
            None => true,
        })
        .filter_map(|m| {
            Some(Listed {
                id: text(m, &["name"])?,
                context_window: number(m, &["context_length"]),
                ..Listed::default()
            })
        })
        .collect();
    Ok(Page {
        models,
        next: text(v, &["next_page_token"]),
    })
}

fn parse_bedrock(v: &Value) -> Result<Page, ParseError> {
    let models = array(v, "modelSummaries")?
        .iter()
        .filter(|m| {
            let out = strings(m.get("outputModalities"));
            out.is_empty() || out.iter().any(|o| o == "text" || o == "embedding")
        })
        .filter_map(|m| {
            let input = strings(m.get("inputModalities"));
            let output = strings(m.get("outputModalities"));
            Some(Listed {
                id: text(m, &["modelId"])?,
                name: text(m, &["modelName"]),
                modalities: (!input.is_empty() || !output.is_empty()).then_some((input, output)),
                ..Listed::default()
            })
        })
        .collect();
    Ok(Page { models, next: None })
}

/// Reads one page of a list in the shape of `listing`. Entries the provider marks as neither
/// text generation nor embeddings are left out.
///
/// # Errors
/// When the answer is not JSON or has no list of models.
pub fn parse(listing: Listing, body: &[u8]) -> Result<Page, ParseError> {
    let v: Value = serde_json::from_slice(body).map_err(|_| ParseError("the answer is not JSON"))?;
    match listing {
        Listing::OpenAi => parse_openai(&v),
        Listing::Azure => parse_azure(&v),
        Listing::Mistral => parse_mistral(&v),
        Listing::Anthropic => parse_anthropic(&v),
        Listing::Gemini => parse_gemini(&v),
        Listing::Cohere => parse_cohere(&v),
        Listing::Bedrock => parse_bedrock(&v),
    }
}

/// How a credential test proved the credential.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TestMethod {
    ListModels,
    MinimalCall,
}

impl TestMethod {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            TestMethod::ListModels => "list_models",
            TestMethod::MinimalCall => "minimal_call",
        }
    }
}

/// The outcome of a provider's latest credential test with one key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestRecord {
    /// The key's name; `None` for a method without named keys (cloud credentials, no auth).
    pub key: Option<String>,
    pub ok: bool,
    pub method: TestMethod,
    pub at: UtcMicros,
    /// Why it failed, scrubbed of the provider's secrets.
    pub error: Option<String>,
}

/// A provider's model list as a node fetched it.
#[derive(Clone, Debug)]
pub struct Fetched {
    /// The provider's configuration it was fetched with.
    pub fingerprint: u64,
    pub at: UtcMicros,
    pub models: Arc<Vec<Listed>>,
}

/// What a node remembers of its credential tests and of the model lists it fetched, by provider.
#[derive(Debug, Default)]
pub struct ProviderChecks {
    tests: Mutex<HashMap<String, BTreeMap<String, TestRecord>>>,
    lists: Mutex<HashMap<String, Fetched>>,
}

impl ProviderChecks {
    /// Records a test's outcome; it replaces the last one of the same key.
    pub fn record_test(&self, provider: &str, record: TestRecord) {
        let key = record.key.clone().unwrap_or_default();
        self.tests
            .lock()
            .entry(provider.to_owned())
            .or_default()
            .insert(key, record);
    }

    /// The latest test of each key of a provider, by key name.
    #[must_use]
    pub fn tests(&self, provider: &str) -> Vec<TestRecord> {
        self.tests
            .lock()
            .get(provider)
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
    }

    /// The provider's model list fetched less than `max_age` ago with the same configuration.
    #[must_use]
    pub fn list(
        &self,
        provider: &str,
        fingerprint: u64,
        now: UtcMicros,
        max_age: Duration,
    ) -> Option<Fetched> {
        self.lists
            .lock()
            .get(provider)
            .filter(|f| f.fingerprint == fingerprint && now.since(f.at) < max_age)
            .cloned()
    }

    /// Keeps a model list just fetched.
    pub fn store_list(&self, provider: &str, fetched: Fetched) {
        self.lists.lock().insert(provider.to_owned(), fetched);
    }

    pub fn clear(&self) {
        self.tests.lock().clear();
        self.lists.lock().clear();
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn ids(p: &Page) -> Vec<&str> {
        p.models.iter().map(|m| m.id.as_str()).collect()
    }

    fn page(listing: Listing, v: &Value) -> Page {
        parse(listing, v.to_string().as_bytes()).unwrap()
    }

    #[test]
    fn openai_lists_and_compatible_extras() {
        let openai = json!({"object": "list", "data": [
            {"id": "gpt-4o", "object": "model", "created": 1715367049, "owned_by": "system"},
            {"id": "text-embedding-3-small", "object": "model", "created": 1705948997, "owned_by": "system"},
            {"id": "dall-e-3", "object": "model", "created": 1698785189, "owned_by": "system"}
        ]});
        let p = page(Listing::OpenAi, &openai);
        assert_eq!(
            ids(&p),
            ["gpt-4o", "text-embedding-3-small", "dall-e-3"],
            "OpenAI marks nothing"
        );
        assert_eq!(p.next, None);
        let together = json!({"data": [
            {"id": "meta-llama/Llama-3.3-70B-Instruct-Turbo", "type": "chat", "display_name": "Llama 3.3 70B",
             "context_length": 131072},
            {"id": "BAAI/bge-large-en-v1.5", "type": "embedding", "context_length": 512},
            {"id": "black-forest-labs/FLUX.1-schnell", "type": "image"},
            {"id": "openai/whisper-large-v3", "type": "transcribe"}
        ]});
        let p = page(Listing::OpenAi, &together);
        assert_eq!(
            ids(&p),
            [
                "meta-llama/Llama-3.3-70B-Instruct-Turbo",
                "BAAI/bge-large-en-v1.5"
            ]
        );
        assert_eq!(p.models[0].name.as_deref(), Some("Llama 3.3 70B"));
        assert_eq!(p.models[0].context_window, Some(131_072));
        let groq = json!({"object": "list", "data": [{"id": "llama-3.1-8b-instant", "context_window": 131072,
                                                      "active": true}]});
        assert_eq!(
            page(Listing::OpenAi, &groq).models[0].context_window,
            Some(131_072)
        );
        assert!(parse(Listing::OpenAi, b"{\"error\": {}}").is_err());
        assert!(parse(Listing::OpenAi, b"not json").is_err());
    }

    #[test]
    fn azure_lists_the_resource_models_it_can_serve() {
        let v = json!({"data": [
            {"id": "gpt-4o-2024-08-06", "capabilities": {"chat_completion": true, "completion": false,
             "embeddings": false, "fine_tune": false, "inference": true}, "lifecycle_status": "generally-available"},
            {"id": "text-embedding-3-large", "capabilities": {"chat_completion": false, "completion": false,
             "embeddings": true, "inference": true}},
            {"id": "dall-e-3", "capabilities": {"chat_completion": false, "completion": false, "embeddings": false,
             "inference": true}}
        ], "object": "list"});
        assert_eq!(
            ids(&page(Listing::Azure, &v)),
            ["gpt-4o-2024-08-06", "text-embedding-3-large"]
        );
    }

    #[test]
    fn mistral_keeps_chat_fim_and_embeddings() {
        let v = json!({"object": "list", "data": [
            {"id": "mistral-large-latest", "name": "mistral-large-2411", "max_context_length": 131072,
             "capabilities": {"completion_chat": true, "completion_fim": false, "function_calling": true,
                              "vision": false}},
            {"id": "codestral-latest", "capabilities": {"completion_chat": true, "completion_fim": true}},
            {"id": "mistral-embed", "capabilities": {"completion_chat": false, "completion_fim": false}},
            {"id": "mistral-ocr-latest", "capabilities": {"completion_chat": false, "ocr": true}},
            {"id": "mistral-moderation-latest", "capabilities": {"completion_chat": false, "classification": true}}
        ]});
        let p = page(Listing::Mistral, &v);
        assert_eq!(
            ids(&p),
            ["mistral-large-latest", "codestral-latest", "mistral-embed"]
        );
        assert_eq!(p.models[0].context_window, Some(131_072));
        assert_eq!(p.models[0].name.as_deref(), Some("mistral-large-2411"));
    }

    #[test]
    fn anthropic_pages_by_last_id() {
        let first = json!({"data": [
            {"type": "model", "id": "claude-opus-4-1-20250805", "display_name": "Claude Opus 4.1",
             "created_at": "2025-08-05T00:00:00Z"},
            {"type": "model", "id": "claude-haiku-4-5-20251001", "display_name": "Claude Haiku 4.5",
             "created_at": "2025-10-01T00:00:00Z", "max_input_tokens": 200000, "max_tokens": 64000}
        ], "has_more": true, "first_id": "claude-opus-4-1-20250805", "last_id": "claude-haiku-4-5-20251001"});
        let p = page(Listing::Anthropic, &first);
        assert_eq!(p.next.as_deref(), Some("claude-haiku-4-5-20251001"));
        assert_eq!(p.models[0].name.as_deref(), Some("Claude Opus 4.1"));
        assert_eq!(
            (p.models[1].context_window, p.models[1].max_output_tokens),
            (Some(200_000), Some(64_000))
        );
        let last = json!({"data": [], "has_more": false, "first_id": null, "last_id": null});
        assert_eq!(page(Listing::Anthropic, &last).next, None);
    }

    #[test]
    fn gemini_keeps_generation_and_embedding_models() {
        let v = json!({"models": [
            {"name": "models/gemini-2.5-flash", "displayName": "Gemini 2.5 Flash", "inputTokenLimit": 1048576,
             "outputTokenLimit": 65536, "supportedGenerationMethods": ["generateContent", "countTokens",
             "createCachedContent", "batchGenerateContent"]},
            {"name": "models/gemini-embedding-001", "displayName": "Gemini Embedding 001",
             "supportedGenerationMethods": ["embedContent", "countTextTokens"]},
            {"name": "models/imagen-4.0-generate-001", "supportedGenerationMethods": ["predict"]},
            {"name": "models/aqa", "supportedGenerationMethods": ["generateAnswer"]}
        ], "nextPageToken": "Ch1tb2RlbHMv"});
        let p = page(Listing::Gemini, &v);
        assert_eq!(ids(&p), ["gemini-2.5-flash", "gemini-embedding-001"]);
        assert_eq!(p.next.as_deref(), Some("Ch1tb2RlbHMv"));
        assert_eq!(p.models[0].context_window, Some(1_048_576));
        assert_eq!(p.models[0].max_output_tokens, Some(65_536));
        assert_eq!(page(Listing::Gemini, &json!({})).models.len(), 0);
        assert!(parse(Listing::Gemini, b"[]").is_err());
    }

    #[test]
    fn cohere_keeps_chat_embed_and_rerank() {
        let v = json!({"models": [
            {"name": "command-a-03-2025", "endpoints": ["chat"], "finetuned": false, "context_length": 256000},
            {"name": "embed-v4.0", "endpoints": ["embed"], "context_length": 128000},
            {"name": "rerank-v3.5", "endpoints": ["rerank"]},
            {"name": "c4ai-aya-vision", "endpoints": ["classify"]}
        ], "next_page_token": ""});
        let p = page(Listing::Cohere, &v);
        assert_eq!(ids(&p), ["command-a-03-2025", "embed-v4.0", "rerank-v3.5"]);
        assert_eq!(p.next, None, "an empty token is the last page");
        assert_eq!(p.models[0].context_window, Some(256_000));
    }

    #[test]
    fn bedrock_keeps_text_and_embedding_models() {
        let v = json!({"modelSummaries": [
            {"modelArn": "arn:aws:bedrock:us-east-1::foundation-model/anthropic.claude-3-5-haiku-20241022-v1:0",
             "modelId": "anthropic.claude-3-5-haiku-20241022-v1:0", "modelName": "Claude 3.5 Haiku",
             "providerName": "Anthropic", "inputModalities": ["TEXT"], "outputModalities": ["TEXT"],
             "responseStreamingSupported": true, "inferenceTypesSupported": ["INFERENCE_PROFILE"]},
            {"modelId": "amazon.titan-embed-text-v2:0", "modelName": "Titan Text Embeddings V2",
             "inputModalities": ["TEXT"], "outputModalities": ["EMBEDDING"]},
            {"modelId": "amazon.nova-canvas-v1:0", "modelName": "Nova Canvas",
             "inputModalities": ["TEXT", "IMAGE"], "outputModalities": ["IMAGE"]}
        ]});
        let p = page(Listing::Bedrock, &v);
        assert_eq!(
            ids(&p),
            [
                "anthropic.claude-3-5-haiku-20241022-v1:0",
                "amazon.titan-embed-text-v2:0"
            ]
        );
        assert_eq!(
            p.models[0].modalities,
            Some((vec!["text".to_owned()], vec!["text".to_owned()]))
        );
    }

    #[test]
    fn listing_requests_per_type() {
        let doc = json!({
            "providers": {
                "o": {"type": "openai", "auth": {"api_keys": [{"name": "a", "key": "sk-a"}, {"name": "b", "key": "sk-b"}]}},
                "z": {"type": "azure_openai", "base_url": "https://r.openai.azure.com",
                      "azure": {"api_style": "deployments", "api_version": "2024-10-21"}, "auth": {"api_key": "k"}},
                "a": {"type": "anthropic", "auth": {"api_key": "sk-ant"}},
                "g": {"type": "gemini", "auth": {"api_key": "AIza"}},
                "c": {"type": "cohere", "auth": {"api_key": "co"}},
                "b": {"type": "bedrock", "aws": {"region": "eu-west-1"}},
                "v": {"type": "vertex", "auth": {"method": "api_key", "api_key": "k"}},
                "m": {"type": "openai_compatible", "base_url": "http://mock:19401/v1", "auth": {"method": "none"}}
            }
        });
        let t = crate::compile(&doc).unwrap();
        let rid = RequestId::generate();
        let url = |name: &str, key: Option<&str>, page: Option<&str>| {
            let p = &t.providers[name];
            let d = DeploymentRt::direct(p, "", "", None);
            let r = d
                .listing_request(p.listing().unwrap(), key, &rid, page, Duration::from_secs(5))
                .unwrap();
            assert_eq!(r.method, Method::GET);
            (r.uri.to_string(), r.headers)
        };
        let (u, hs) = url("o", Some("b"), None);
        assert_eq!(u, "https://api.openai.com/v1/models");
        assert_eq!(hs["authorization"], "Bearer sk-b");
        assert_eq!(
            url("z", None, None).0,
            "https://r.openai.azure.com/openai/models?api-version=2024-10-21"
        );
        let (u, hs) = url("a", None, Some("claude-x"));
        assert_eq!(
            u,
            "https://api.anthropic.com/v1/models?limit=1000&after_id=claude-x"
        );
        assert!(hs.contains_key("anthropic-version"));
        assert!(url("g", None, None).0.ends_with("/models?pageSize=1000"));
        assert_eq!(
            url("c", None, Some("t")).0,
            "https://api.cohere.com/v1/models?page_size=1000&page_token=t"
        );
        assert_eq!(
            url("b", None, None).0,
            "https://bedrock.eu-west-1.amazonaws.com/foundation-models"
        );
        assert_eq!(url("m", None, None).0, "http://mock:19401/v1/models");
        assert_eq!(t.providers["v"].listing(), None);
    }

    #[test]
    fn checks_keep_the_latest_test_per_key_and_fresh_lists() {
        let c = ProviderChecks::default();
        let rec = |key: Option<&str>, ok: bool, at: i64| TestRecord {
            key: key.map(str::to_owned),
            ok,
            method: TestMethod::ListModels,
            at: UtcMicros(at),
            error: (!ok).then(|| "no".to_owned()),
        };
        c.record_test("p", rec(Some("a"), false, 1));
        c.record_test("p", rec(Some("a"), true, 2));
        c.record_test("p", rec(Some("b"), false, 3));
        let tests = c.tests("p");
        assert_eq!(tests.len(), 2);
        assert!(tests[0].ok && !tests[1].ok);
        assert!(c.tests("q").is_empty());
        c.store_list(
            "p",
            Fetched {
                fingerprint: 7,
                at: UtcMicros(0),
                models: Arc::new(Vec::new()),
            },
        );
        let min = Duration::from_secs(60);
        assert!(c.list("p", 7, UtcMicros(1_000_000), min).is_some());
        assert!(
            c.list("p", 8, UtcMicros(1_000_000), min).is_none(),
            "another configuration"
        );
        assert!(c.list("p", 7, UtcMicros(61_000_000), min).is_none(), "too old");
        c.clear();
        assert!(c.tests("p").is_empty());
    }
}
