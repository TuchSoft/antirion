//! The OpenAI Decisions codec: decode the client request into the IR and encode a System One answer
//! back into the OpenAI answer array.

use serde_json::{Map, Value, json};

use super::ir::{
    Choice, ChoiceValue, DecideMap, Decoded, Input, Instructions, Level, Question, QuestionKind, Request,
};
use super::refusal;
use crate::xlate::warn::Refusal;

/// Decodes an OpenAI `/v1/decisions` body.
pub(super) fn decode(body: &Value) -> Result<Decoded, Refusal> {
    let obj = body
        .as_object()
        .ok_or_else(|| refusal("", "The request body must be an object."))?;
    let input = match obj.get("input") {
        Some(Value::String(s)) => Input::Text(s.clone()),
        Some(Value::Array(items)) => Input::Messages(items.clone()),
        _ => return Err(refusal("input", "must be a string or an array of input items.")),
    };
    let list = obj
        .get("questions")
        .and_then(Value::as_array)
        .ok_or_else(|| refusal("questions", "must be an array."))?;
    let mut questions = Vec::with_capacity(list.len());
    for (i, item) in list.iter().enumerate() {
        let path = format!("questions[{i}]");
        let q = item
            .as_object()
            .ok_or_else(|| refusal(&path, "must be an object."))?;
        let name = q.get("name").and_then(Value::as_str).map(str::to_owned);
        let instructions = match q.get("instructions") {
            Some(Value::String(s)) => Instructions::Text(s.clone()),
            Some(Value::Array(parts)) => Instructions::Parts(parts.clone()),
            _ => {
                return Err(refusal(
                    &format!("{path}.instructions"),
                    "must be a string or an array of content parts.",
                ));
            }
        };
        let kind = match q.get("type").and_then(Value::as_str) {
            Some("predicate") => QuestionKind::Predicate,
            Some("choice") => QuestionKind::Choice(choices(q, &path)?),
            Some("score") => QuestionKind::Score(levels(q, &path)?),
            _ => {
                return Err(refusal(
                    &format!("{path}.type"),
                    "is not a decision question type.",
                ));
            }
        };
        questions.push(Question {
            name,
            instructions,
            kind,
        });
    }
    Ok(Decoded {
        request: Request { input, questions },
    })
}

fn choices(q: &Map<String, Value>, path: &str) -> Result<Vec<Choice>, Refusal> {
    let Some(Value::Array(list)) = q.get("choices") else {
        return Err(refusal(&format!("{path}.choices"), "must be an array."));
    };
    let mut out = Vec::with_capacity(list.len());
    for (j, item) in list.iter().enumerate() {
        let at = format!("{path}.choices[{j}]");
        let choice = item
            .as_object()
            .ok_or_else(|| refusal(&at, "must be an object."))?;
        let value = choice
            .get("value")
            .and_then(ChoiceValue::from_json)
            .ok_or_else(|| refusal(&format!("{at}.value"), "must be a string or a boolean."))?;
        let description = choice
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned);
        out.push(Choice { value, description });
    }
    Ok(out)
}

fn levels(q: &Map<String, Value>, path: &str) -> Result<Vec<Level>, Refusal> {
    let Some(Value::Array(list)) = q.get("levels") else {
        return Err(refusal(&format!("{path}.levels"), "must be an array."));
    };
    let mut out = Vec::with_capacity(list.len());
    for (j, item) in list.iter().enumerate() {
        let at = format!("{path}.levels[{j}]");
        let level = item
            .as_object()
            .ok_or_else(|| refusal(&at, "must be an object."))?;
        let label = level
            .get("label")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| refusal(&format!("{at}.label"), "must be a string."))?;
        let description = level
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned);
        out.push(Level { label, description });
    }
    Ok(out)
}

/// Encodes a System One answer in the OpenAI answer array shape.
pub(super) fn encode_response(upstream: &Value, map: &DecideMap, shown: &str) -> Value {
    let answers = upstream.get("answers").and_then(Value::as_object);
    let out: Vec<Value> = map
        .questions
        .iter()
        .enumerate()
        .map(|(i, question)| {
            let key = map.keys.get(i).map_or("", String::as_str);
            let answer = answers.and_then(|a| a.get(key)).and_then(Value::as_object);
            answer_value(question, answer)
        })
        .collect();
    let usage = upstream.get("usage").and_then(Value::as_object);
    let input = usage
        .and_then(|u| u.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = usage
        .and_then(|u| u.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    json!({
        "model": shown,
        "answers": out,
        "usage": {
            "input_tokens": input,
            "input_tokens_details": {"cached_tokens": 0, "cache_write_tokens": 0},
            "output_tokens": output,
            "output_tokens_details": {"reasoning_tokens": 0},
            "total_tokens": input.saturating_add(output),
        },
    })
}

fn answer_value(question: &Question, answer: Option<&Map<String, Value>>) -> Value {
    let Some(answer) = answer else {
        return declined(question);
    };
    match (&question.kind, answer.get("type").and_then(Value::as_str)) {
        (QuestionKind::Predicate, Some("noul")) => {
            let Some(probability) = answer.get("noul").and_then(Value::as_f64) else {
                return declined(question);
            };
            let mut out = Map::new();
            out.insert("type".into(), Value::from("predicate"));
            out.insert("name".into(), name_value(question));
            out.insert("probability".into(), Value::from(probability));
            Value::Object(out)
        }
        (QuestionKind::Choice(options), Some("choice")) => {
            let Some(choice) = answer.get("choice").and_then(Value::as_str) else {
                return declined(question);
            };
            if !options.iter().any(|o| o.value.key() == choice) {
                return declined(question);
            }
            let probabilities = answer.get("probabilities").and_then(Value::as_object);
            let emitted: Vec<Value> = options
                .iter()
                .map(|option| {
                    let key = option.value.key();
                    let probability = probabilities
                        .and_then(|p| p.get(&key))
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0);
                    json!({"value": option.value.to_json(), "probability": probability})
                })
                .collect();
            let mut out = Map::new();
            out.insert("type".into(), Value::from("choice"));
            out.insert("name".into(), name_value(question));
            out.insert("choice".into(), Value::String(choice.to_owned()));
            out.insert("probabilities".into(), Value::Array(emitted));
            if let Some(confidence) = answer.get("confidence").and_then(Value::as_f64) {
                out.insert("confidence".into(), Value::from(confidence));
            }
            Value::Object(out)
        }
        (QuestionKind::Score(levels), Some("score")) => {
            let Some(score) = answer.get("score").and_then(Value::as_f64) else {
                return declined(question);
            };
            let probabilities = answer.get("probabilities").and_then(Value::as_object);
            let emitted: Vec<Value> = levels
                .iter()
                .enumerate()
                .map(|(index, level)| {
                    let key = index.to_string();
                    let probability = probabilities
                        .and_then(|p| p.get(&key))
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0);
                    json!({
                        "value": index,
                        "label": level.label,
                        "probability": probability,
                    })
                })
                .collect();
            let mut out = Map::new();
            out.insert("type".into(), Value::from("score"));
            out.insert("name".into(), name_value(question));
            out.insert("score".into(), Value::from(score));
            out.insert("probabilities".into(), Value::Array(emitted));
            if let Some(confidence) = answer.get("confidence").and_then(Value::as_f64) {
                out.insert("confidence".into(), Value::from(confidence));
            }
            Value::Object(out)
        }
        _ => declined(question),
    }
}

fn name_value(question: &Question) -> Value {
    question.name.clone().map_or(Value::Null, Value::String)
}

fn declined(question: &Question) -> Value {
    json!({"type": "refusal", "name": name_value(question)})
}
