//! The System One codec: check that an OpenAI-shaped decision can be carried, encode its request,
//! and leave the answer mapping to the OpenAI encoder.

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::ir::{DecideMap, Instructions, Question, QuestionKind, Request};
use super::refusal;
use crate::xlate::warn::Refusal;

/// The most score levels System One accepts.
const MAX_SCORE_LEVELS: usize = 10;

/// Whether the request can be carried by System One.
pub(super) fn feasible(req: &Request) -> Result<(), Refusal> {
    check_input(req)?;
    for (i, question) in req.questions.iter().enumerate() {
        let path = format!("questions[{i}]");
        check_instructions(&question.instructions, &format!("{path}.instructions"))?;
        match &question.kind {
            QuestionKind::Predicate => {}
            QuestionKind::Choice(options) => {
                let mut seen: HashSet<String> = HashSet::new();
                for (j, option) in options.iter().enumerate() {
                    if !seen.insert(option.value.key()) {
                        return Err(refusal(
                            &format!("{path}.choices[{j}].value"),
                            "choice values must be unique as text for this model.",
                        ));
                    }
                }
            }
            QuestionKind::Score(levels) => {
                if levels.len() > MAX_SCORE_LEVELS {
                    return Err(refusal(
                        &format!("{path}.levels"),
                        format!("this model accepts at most {MAX_SCORE_LEVELS} score levels per question."),
                    ));
                }
                if levels.len() < 2 {
                    return Err(refusal(
                        &format!("{path}.levels"),
                        "this model accepts at least two score levels per question.",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn check_input(req: &Request) -> Result<(), Refusal> {
    let super::ir::Input::Messages(items) = &req.input else {
        return Ok(());
    };
    for (i, item) in items.iter().enumerate() {
        let path = format!("input[{i}]");
        let Some(obj) = item.as_object() else {
            return Err(refusal(&path, "must be an input object."));
        };
        if obj.get("role").and_then(Value::as_str) != Some("user") {
            return Err(refusal(&path, "this decision model accepts user messages only."));
        }
        match obj.get("content") {
            Some(Value::String(_)) => {}
            Some(Value::Array(parts)) => check_parts(parts, &format!("{path}.content"))?,
            _ => {
                return Err(refusal(
                    &format!("{path}.content"),
                    "must be a string or an array of input_text parts.",
                ));
            }
        }
    }
    Ok(())
}

fn check_instructions(instructions: &Instructions, path: &str) -> Result<(), Refusal> {
    match instructions {
        Instructions::Text(_) => Ok(()),
        Instructions::Parts(parts) => check_parts(parts, path),
    }
}

fn check_parts(parts: &[Value], path: &str) -> Result<(), Refusal> {
    for (i, part) in parts.iter().enumerate() {
        let at = format!("{path}[{i}]");
        let Some(obj) = part.as_object() else {
            return Err(refusal(&at, "must be a content part."));
        };
        if obj.get("type").and_then(Value::as_str) != Some("input_text") {
            return Err(refusal(&at, "this decision model accepts input_text parts only."));
        }
    }
    Ok(())
}

/// Encodes the request for System One.
pub(super) fn encode(req: &Request, upstream_model: &str) -> Result<(Value, DecideMap), Refusal> {
    feasible(req)?;
    let keys = question_keys(&req.questions)?;
    let state = match &req.input {
        super::ir::Input::Text(text) => Value::String(text.clone()),
        super::ir::Input::Messages(items) => Value::String(flatten_messages(items)),
        super::ir::Input::State(state) => state.clone(),
    };
    let mut questions = Map::new();
    for (i, question) in req.questions.iter().enumerate() {
        let key = keys.get(i).cloned().unwrap_or_default();
        questions.insert(key, encode_question(question));
    }
    let body = json!({
        "model": upstream_model,
        "state": state,
        "questions": Value::Object(questions),
    });
    Ok((
        body,
        DecideMap {
            keys,
            questions: req.questions.clone(),
        },
    ))
}

fn encode_question(question: &Question) -> Value {
    let instructions = instructions_value(&question.instructions);
    match &question.kind {
        QuestionKind::Predicate => json!({"type": "noul", "instructions": instructions}),
        QuestionKind::Choice(options) => {
            let mut criteria = Map::new();
            for option in options {
                criteria.insert(
                    option.value.key(),
                    option.description.clone().map_or(Value::Null, Value::String),
                );
            }
            json!({
                "type": "choice",
                "instructions": instructions,
                "criteria": Value::Object(criteria),
            })
        }
        QuestionKind::Score(levels) => {
            let criteria: Vec<Value> = levels
                .iter()
                .map(|level| {
                    Value::String(match &level.description {
                        Some(description) => format!("{}: {description}", level.label),
                        None => level.label.clone(),
                    })
                })
                .collect();
            json!({
                "type": "score",
                "instructions": instructions,
                "criteria": criteria,
            })
        }
    }
}

fn instructions_value(instructions: &Instructions) -> Value {
    match instructions {
        Instructions::Text(text) => Value::String(text.clone()),
        Instructions::Parts(parts) => {
            let texts: Vec<&str> = parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect();
            Value::String(texts.join("\n"))
        }
    }
}

fn flatten_messages(items: &[Value]) -> String {
    let messages: Vec<String> = items
        .iter()
        .map(|item| match item.get("content") {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Array(parts)) => parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<&str>>()
                .join("\n"),
            _ => String::new(),
        })
        .collect();
    messages.join("\n\n")
}

fn question_keys(questions: &[Question]) -> Result<Vec<String>, Refusal> {
    let mut named: HashSet<&str> = HashSet::new();
    for (i, question) in questions.iter().enumerate() {
        if let Some(name) = question.name.as_deref()
            && !named.insert(name)
        {
            return Err(refusal(
                &format!("questions[{i}].name"),
                "question names must be unique.",
            ));
        }
    }
    let mut taken: HashSet<String> = named.iter().map(|n| (*n).to_owned()).collect();
    let mut keys = Vec::with_capacity(questions.len());
    for (i, question) in questions.iter().enumerate() {
        if let Some(name) = &question.name {
            keys.push(name.clone());
            continue;
        }
        let mut key = format!("question_{}", i.saturating_add(1));
        while taken.contains(&key) {
            key.insert(0, '_');
        }
        taken.insert(key.clone());
        keys.push(key);
    }
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::decide::ir::{Choice, ChoiceValue, Level};

    fn options(values: &[&str]) -> Vec<Choice> {
        values
            .iter()
            .map(|v| Choice {
                value: ChoiceValue::Str((*v).to_owned()),
                description: None,
            })
            .collect()
    }

    #[test]
    fn unnamed_keys_do_not_collide() {
        let questions = vec![
            Question {
                name: None,
                instructions: Instructions::Text("?".into()),
                kind: QuestionKind::Predicate,
            },
            Question {
                name: Some("question_1".into()),
                instructions: Instructions::Text("?".into()),
                kind: QuestionKind::Predicate,
            },
        ];
        assert_eq!(
            question_keys(&questions).unwrap_or_default(),
            ["_question_1", "question_1"]
        );
    }

    #[test]
    fn choices_are_encoded_as_criteria() {
        let question = Question {
            name: Some("pick".into()),
            instructions: Instructions::Text("Pick".into()),
            kind: QuestionKind::Choice(options(&["constructor", "__proto__"])),
        };
        let out = encode_question(&question);
        assert_eq!(out.pointer("/criteria/constructor"), Some(&Value::Null), "{out}");
        assert_eq!(out.pointer("/criteria/__proto__"), Some(&Value::Null), "{out}");
    }

    #[test]
    fn score_criteria_keep_the_label_and_description() {
        let question = Question {
            name: Some("severity".into()),
            instructions: Instructions::Text("How bad?".into()),
            kind: QuestionKind::Score(vec![
                Level {
                    label: "Low".into(),
                    description: Some("Cosmetic".into()),
                },
                Level {
                    label: "High".into(),
                    description: None,
                },
            ]),
        };
        let out = encode_question(&question);
        assert_eq!(out.get("criteria"), Some(&json!(["Low: Cosmetic", "High"])));
    }
}
