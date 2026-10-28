//! Tool search: a gateway that lists only three tools, which find the granted tools, show one
//! tool's definition and call one, checking its arguments against the tool's input schema before
//! any server is contacted. The search ranks by words of a tool's name and description, or, when
//! the gateway names an embedding model, by meaning.

use std::sync::Arc;
use std::time::{Duration, Instant};

use antirion_config::schema::UserSchema;
use serde_json::{Value, json};

use crate::compile::{SemanticRt, ServerRt, ToolSearchRt};
use crate::hooks::Pending;
use crate::http::Req;
use crate::lists::{self, Kind, Mode};
use crate::names;
use crate::rpc::Fail;
use crate::select::Selection;

pub(crate) const SEARCH: &str = "search_tools";
pub(crate) const SCHEMA: &str = "get_tool_schema";
pub(crate) const CALL: &str = "call_tool";

const DEFAULT_LIMIT: usize = 10;
const MAX_LIMIT: usize = 50;

/// The embeddings a semantic tool search needs: model calls, vector caches and durable vectors are
/// the node's business, not the gateway's.
pub trait ToolIndexBackend: Send + Sync + std::fmt::Debug {
    /// Vectors for tool texts, in order; `None` for a text that could not be vectorised.
    /// Loads cached vectors, embeds the misses and persists them.
    #[allow(clippy::type_complexity)]
    fn tool_vectors<'a>(
        &'a self,
        model: &'a str,
        texts: &'a [String],
        timeout: Duration,
    ) -> Pending<'a, Result<Vec<Option<Arc<[f32]>>>, String>>;

    /// The query's vector. Never cached durably; a node-local LRU at most.
    fn query_vector<'a>(
        &'a self,
        model: &'a str,
        text: &'a str,
        timeout: Duration,
    ) -> Pending<'a, Result<Option<Arc<[f32]>>, String>>;
}

/// Whether `name` is one of the three tools.
pub(crate) fn is_meta(name: &str) -> bool {
    matches!(name, SEARCH | SCHEMA | CALL)
}

/// The definitions `tools/list` answers with, describing the ranking this gateway's mode uses.
pub(crate) fn tools(search: &ToolSearchRt) -> Value {
    let (how, note, limit) = match search {
        ToolSearchRt::Semantic(sem) => (
            "by meaning: it matches the query against what each tool does, not just its words",
            " Each result carries a \"score\": how close the tool is to the query, higher is closer.",
            format!(
                "How many tools to return (default {}, at most {}).",
                sem.top_k, sem.top_k
            ),
        ),
        ToolSearchRt::Off | ToolSearchRt::Lexical => (
            "by words of their name or description",
            "",
            format!("How many tools to return (default {DEFAULT_LIMIT})."),
        ),
    };
    json!([
        {
            "name": SEARCH,
            "description": format!("Finds tools {how}. Returns names and descriptions{note}; read a tool's input schema with get_tool_schema before calling it with call_tool."),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Words to look for."},
                    "limit": {"type": "integer", "minimum": 1, "maximum": MAX_LIMIT, "description": limit}
                },
                "required": ["query"],
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": SCHEMA,
            "description": "Returns one tool's full definition, with its input schema.",
            "inputSchema": {
                "type": "object",
                "properties": {"name": {"type": "string", "description": "The tool's name, as search_tools returned it."}},
                "required": ["name"],
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": CALL,
            "description": "Calls one tool with arguments that satisfy its input schema.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "The tool's name."},
                    "arguments": {"type": "object", "description": "The tool's arguments."}
                },
                "required": ["name"],
                "additionalProperties": false
            }
        }
    ])
}

/// A tool result that reports a problem to the model.
fn error_result(text: &str) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": true})
}

fn json_result(v: &Value) -> Value {
    json!({
        "content": [{"type": "text", "text": v.to_string()}],
        "structuredContent": v,
        "isError": false,
    })
}

/// Every tool the caller may see on the gateway's servers, as `tools/list` would list them.
async fn catalog(
    req: &Req,
    sel: &Selection,
    mode: &Mode,
    servers: &[Arc<ServerRt>],
) -> Result<Vec<Value>, Fail> {
    let modern = matches!(mode, Mode::Modern { .. });
    let read = lists::entries(req, mode, servers, Kind::Tools, modern).await;
    let mut ok = Vec::new();
    let mut failed = Vec::new();
    for (server, r) in read {
        match r {
            Ok(e) => ok.push((server, e)),
            Err(e) => failed.push((server.name.clone(), e)),
        }
    }
    crate::fanout::policy(req.gw.fail_closed, servers.len(), &failed)?;
    Ok(lists::merge(req, sel, Kind::Tools, &ok).items)
}

fn text_of(tool: &Value, key: &str) -> String {
    tool.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase()
}

/// The tools matching `query`: ranked by how many of its words a tool's name or description
/// contains, name matches first, then by name.
fn search(tools: &[Value], query: &str, limit: usize) -> Vec<Value> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut scored: Vec<(usize, usize, String, &Value)> = tools
        .iter()
        .filter_map(|t| {
            let name = text_of(t, "name");
            let description = text_of(t, "description");
            let in_name = terms.iter().filter(|w| name.contains(w.as_str())).count();
            let anywhere = terms
                .iter()
                .filter(|w| name.contains(w.as_str()) || description.contains(w.as_str()))
                .count();
            (anywhere > 0 || terms.is_empty()).then_some((anywhere, in_name, name, t))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then_with(|| a.2.cmp(&b.2)));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, _, _, t)| {
            json!({
                "name": t.get("name"),
                "description": t.get("description").cloned().unwrap_or(Value::Null),
            })
        })
        .collect()
}

/// The upstream name behind a client-facing tool name: namespacing prefixes cannot contain `__`,
/// so a split at the first separator yields exactly the name the server offered.
fn upstream_name(gw_prefixed: bool, client: &str) -> &str {
    if gw_prefixed {
        names::split_name(client).map_or(client, |(_, up)| up)
    } else {
        client
    }
}

/// Cuts `text` to at most `max_chars` characters, on a character boundary.
fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => text.get(..cut).unwrap_or_default().to_owned(),
        None => text.to_owned(),
    }
}

/// The text a tool is embedded as: its `name`, a blank line and its `description`, cut to
/// `max_chars`.
fn tool_text(tool: &Value, max_chars: usize) -> String {
    let name = tool.get("name").and_then(Value::as_str).unwrap_or_default();
    let description = tool
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default();
    truncate(&format!("{name}\n\n{description}"), max_chars)
}

/// The text one catalog entry is embedded as, under the name the server offered: a `prefix_mode`
/// change does not re-embed every tool.
fn entry_text(tool: &Value, gw_prefixed: bool, max_chars: usize) -> String {
    let client = tool.get("name").and_then(Value::as_str).unwrap_or_default();
    let named = json!({
        "name": upstream_name(gw_prefixed, client),
        "description": tool.get("description").cloned().unwrap_or(Value::Null),
    });
    tool_text(&named, max_chars)
}

/// Cosine similarity; 0 for vectors of different lengths or a zero vector.
fn cosine(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0.0f64, 0.0f64, 0.0f64);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (f64::from(*x), f64::from(*y));
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// The tools at least `threshold` close to the query, closest first, at most `limit` of them.
fn rank(
    tools: &[Value],
    vectors: &[Option<Arc<[f32]>>],
    query: &[f32],
    limit: usize,
    threshold: f64,
) -> Vec<Value> {
    let mut scored: Vec<(f64, &Value)> = tools
        .iter()
        .zip(vectors)
        .filter_map(|(tool, vector)| {
            let score = cosine(vector.as_deref()?, query);
            (score >= threshold).then_some((score, tool))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored
        .into_iter()
        .take(limit)
        .map(|(score, tool)| {
            json!({
                "name": tool.get("name"),
                "description": tool.get("description").cloned().unwrap_or(Value::Null),
                "score": score,
            })
        })
        .collect()
}

/// The tools matching `query` by meaning, or the lexical ranking when the embeddings cannot answer.
/// A search never fails because an embedding did.
async fn semantic_search(
    req: &Req,
    tools: &[Value],
    sem: &SemanticRt,
    query: &str,
    limit: Option<usize>,
) -> Vec<Value> {
    let cap = limit.unwrap_or(sem.top_k).min(sem.top_k).max(1);
    let words = || search(tools, query, cap);
    if tools.is_empty() {
        return Vec::new();
    }
    if query.trim().is_empty() {
        return words();
    }
    let Some(backend) = req.env.tool_index.clone() else {
        tracing::debug!(
            gateway = %req.gw.name,
            "no tool index is installed; ranking by words"
        );
        req.env.log.tool_index(|f| f.fallback = true);
        return words();
    };
    let texts: Vec<String> = tools
        .iter()
        .map(|t| entry_text(t, req.gw.prefixed, sem.max_text_chars))
        .collect();
    let started = Instant::now();
    let attempt = async {
        let vectors = backend.tool_vectors(&sem.model, &texts, sem.timeout).await?;
        let Some(vector) = backend.query_vector(&sem.model, query, sem.timeout).await? else {
            return Err("the query has no vector".to_owned());
        };
        Ok(rank(tools, &vectors, &vector, cap, sem.threshold))
    };
    let ranked: Option<Vec<Value>> = tokio::select! {
        _ = req.env.gone.cancelled() => {
            tracing::debug!(gateway = %req.gw.name, "the caller went away before the tools embedded");
            None
        }
        r = tokio::time::timeout(sem.timeout, attempt) => match r {
            Ok(Ok(hits)) => Some(hits),
            Ok(Err(e)) => {
                tracing::debug!(
                    gateway = %req.gw.name,
                    error = %e,
                    "the tool index could not embed; ranking by words"
                );
                None
            }
            Err(_) => {
                tracing::debug!(gateway = %req.gw.name, "the tool search timed out; ranking by words");
                None
            }
        },
    };
    let elapsed = started.elapsed();
    req.env.log.tool_index(|f| {
        f.fallback = ranked.is_none();
        f.elapsed = elapsed;
    });
    ranked.unwrap_or_else(words)
}

/// What a call of one of the three tools asks for.
pub(crate) enum Asked {
    /// The tool's result, answered by the gateway.
    Answer(Value),
    /// A `tools/call` of the named tool with these arguments.
    Call { name: String, arguments: Value },
}

/// Answers `search_tools` and `get_tool_schema`, and checks a `call_tool`.
pub(crate) async fn ask(
    req: &Req,
    sel: &Selection,
    mode: &Mode,
    servers: &[Arc<ServerRt>],
    name: &str,
    args: &Value,
) -> Result<Asked, Fail> {
    let tools = catalog(req, sel, mode, servers).await?;
    let find = |n: &str| {
        tools
            .iter()
            .find(|t| t.get("name").and_then(Value::as_str) == Some(n))
    };
    match name {
        SEARCH => {
            let Some(query) = args.get("query").and_then(Value::as_str) else {
                return Ok(Asked::Answer(error_result("query must be a string.")));
            };
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .and_then(|l| usize::try_from(l).ok())
                .map(|l| l.clamp(1, MAX_LIMIT));
            let found = match &req.gw.tool_search {
                ToolSearchRt::Semantic(sem) => semantic_search(req, &tools, sem, query, limit).await,
                ToolSearchRt::Off | ToolSearchRt::Lexical => {
                    search(&tools, query, limit.unwrap_or(DEFAULT_LIMIT))
                }
            };
            Ok(Asked::Answer(json_result(&json!({"tools": found}))))
        }
        SCHEMA => {
            let wanted = args.get("name").and_then(Value::as_str).unwrap_or_default();
            Ok(Asked::Answer(match find(wanted) {
                Some(t) => json_result(t),
                None => error_result(&format!("There is no tool named {wanted:?}.")),
            }))
        }
        _ => {
            let wanted = args.get("name").and_then(Value::as_str).unwrap_or_default();
            let Some(tool) = find(wanted) else {
                return Ok(Asked::Answer(error_result(&format!(
                    "There is no tool named {wanted:?}."
                ))));
            };
            let arguments = args.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let schema = tool
                .get("inputSchema")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object"}));
            match UserSchema::compile(&schema) {
                Ok(s) => {
                    if let Some((at, why)) = s.first_violation(&arguments) {
                        let at = if at.is_empty() {
                            "arguments".to_owned()
                        } else {
                            format!("arguments.{at}")
                        };
                        return Ok(Asked::Answer(error_result(&format!(
                            "The arguments do not match the input schema of {wanted}: {at}: {why}"
                        ))));
                    }
                }
                Err(e) => {
                    return Ok(Asked::Answer(error_result(&format!(
                        "The input schema of {wanted} cannot be checked: {e}"
                    ))));
                }
            }
            Ok(Asked::Call {
                name: wanted.to_owned(),
                arguments,
            })
        }
    }
}

/// The `params` of the `tools/call` a checked `call_tool` becomes.
pub(crate) fn call_params(original: Option<&Value>, name: &str, arguments: Value) -> Value {
    let mut p = original.cloned().unwrap_or_else(|| json!({}));
    if let Some(o) = p.as_object_mut() {
        o.insert("name".into(), Value::from(name));
        o.insert("arguments".into(), arguments);
    }
    p
}

/// Answers a request of a tool-search gateway, or `None` for one the usual path serves.
pub(crate) async fn intercept(
    req: &Req,
    sel: &Selection,
    mode: &Mode,
    servers: &[Arc<ServerRt>],
    method: &str,
    params: Option<&Value>,
) -> Option<Result<Intercepted, Fail>> {
    if matches!(req.gw.tool_search, ToolSearchRt::Off) {
        return None;
    }
    match method {
        "tools/list" => Some(Ok(Intercepted::Result(
            json!({"tools": tools(&req.gw.tool_search)}),
        ))),
        "tools/call" => {
            let name = params.and_then(|p| p.get("name")).and_then(Value::as_str)?;
            if !is_meta(name) {
                return None;
            }
            let args = params
                .and_then(|p| p.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            Some(ask(req, sel, mode, servers, name, &args).await.map(|a| match a {
                Asked::Answer(v) => Intercepted::Result(v),
                Asked::Call { name, arguments } => {
                    Intercepted::Forward(call_params(params, &name, arguments))
                }
            }))
        }
        _ => None,
    }
}

/// What a tool-search gateway does with a request.
pub(crate) enum Intercepted {
    /// Answer with this result.
    Result(Value),
    /// Forward a `tools/call` with these params.
    Forward(Value),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_ranks_by_matching_words() {
        let tools = vec![
            json!({"name": "weather__forecast", "description": "Daily weather forecast for a city"}),
            json!({"name": "files__read", "description": "Read a file"}),
            json!({"name": "weather__alerts", "description": "Severe weather alerts"}),
        ];
        let hits = search(&tools, "weather forecast", 10);
        let names: Vec<&str> = hits
            .iter()
            .filter_map(|h| h.get("name").and_then(Value::as_str))
            .collect();
        assert_eq!(names, ["weather__forecast", "weather__alerts"]);
        assert_eq!(search(&tools, "FILE", 10).len(), 1);
        assert_eq!(search(&tools, "weather", 1).len(), 1);
        assert!(search(&tools, "nothing", 10).is_empty());
    }

    #[test]
    fn tool_texts_carry_the_upstream_name_and_stop_on_a_char_boundary() {
        let tool = json!({"name": "github__create_issue", "description": "Opens an issue"});
        assert_eq!(tool_text(&tool, 1_000), "github__create_issue\n\nOpens an issue");
        assert_eq!(tool_text(&json!({"name": "x"}), 1_000), "x\n\n");
        let long = json!({"name": "t", "description": "à".repeat(10)});
        assert_eq!(tool_text(&long, 0), "");
        assert_eq!(tool_text(&long, 3), "t\n\n");
        let cut = tool_text(&long, 5);
        assert_eq!(cut.chars().count(), 5);
        assert_eq!(cut, "t\n\nàà");
    }

    #[test]
    fn upstream_names_survive_a_separator_inside_them() {
        assert_eq!(upstream_name(true, "github__create_issue"), "create_issue");
        assert_eq!(upstream_name(true, "a__get__raw"), "get__raw");
        assert_eq!(upstream_name(true, "plain"), "plain");
        assert_eq!(upstream_name(false, "get__raw"), "get__raw");
        assert_eq!(upstream_name(false, "files__read_deep"), "files__read_deep");
        assert_eq!(
            entry_text(&json!({"name": "a__get__raw", "description": "Reads"}), true, 100),
            "get__raw\n\nReads"
        );
    }

    #[test]
    fn semantic_ranking_drops_weak_and_extra_tools() {
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 0.0]), 0.0);
        assert_eq!(cosine(&[1.0], &[1.0, 0.0]), 0.0);
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < f64::EPSILON);
        let tools = vec![
            json!({"name": "a", "description": "first"}),
            json!({"name": "b", "description": "second"}),
            json!({"name": "c", "description": "third"}),
        ];
        let vectors = vec![
            Some(Arc::<[f32]>::from(vec![1.0, 0.0])),
            None,
            Some(Arc::<[f32]>::from(vec![0.0, 1.0])),
        ];
        let hits = rank(&tools, &vectors, &[1.0, 0.0], 10, 0.5);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits.first().and_then(|h| h.get("name")).and_then(Value::as_str),
            Some("a")
        );
        let ordered = rank(&tools, &vectors, &[0.6, 0.8], 10, 0.5);
        let names: Vec<&str> = ordered
            .iter()
            .filter_map(|h| h.get("name").and_then(Value::as_str))
            .collect();
        assert_eq!(names, ["c", "a"]);
        assert_eq!(rank(&tools, &vectors, &[1.0, 0.0], 1, 0.0).len(), 1);
    }

    #[test]
    fn the_three_tools() {
        let t = tools(&ToolSearchRt::Lexical);
        let names: Vec<&str> = t
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x.get("name").and_then(Value::as_str))
            .collect();
        assert_eq!(names, [SEARCH, SCHEMA, CALL]);
        let p = call_params(
            Some(&json!({"name": CALL, "_meta": {"k": 1}})),
            "a__b",
            json!({"x": 1}),
        );
        assert_eq!(
            p,
            json!({"name": "a__b", "arguments": {"x": 1}, "_meta": {"k": 1}})
        );
    }

    #[test]
    fn semantic_gateways_describe_their_search() {
        let semantic = tools(&ToolSearchRt::Semantic(Arc::new(SemanticRt {
            model: "embed".into(),
            threshold: 0.3,
            top_k: 5,
            timeout: Duration::from_secs(1),
            max_text_chars: 100,
        })));
        let search_tool = semantic
            .as_array()
            .into_iter()
            .flatten()
            .find(|x| x.get("name").and_then(Value::as_str) == Some(SEARCH));
        let description = search_tool
            .and_then(|x| x.get("description"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(description.contains("by meaning"));
        assert!(description.contains("score"));
        let limit = search_tool
            .and_then(|x| x.pointer("/inputSchema/properties/limit/description"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(limit.contains("default 5"));
        let lexical = tools(&ToolSearchRt::Off);
        let description = lexical
            .as_array()
            .into_iter()
            .flatten()
            .find(|x| x.get("name").and_then(Value::as_str) == Some(SEARCH))
            .and_then(|x| x.get("description"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(description.contains("by words of their name or description"));
    }
}
