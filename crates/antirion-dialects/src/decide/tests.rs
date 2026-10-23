//! Translation parity tests for the decision codecs.

use serde_json::{Value, json};

use super::ir::{ChoiceValue, DecisionTarget, Instructions, QuestionKind};
use super::{decode, encode, feasible, response};

fn decode_body(body: &Value) -> super::Decoded {
    decode(body).unwrap()
}

fn translated(body: Value) -> Value {
    let decoded = decode_body(&body);
    encode(DecisionTarget::SystemOne, &decoded, "upstream-model")
        .unwrap()
        .0
}

#[test]
fn text_and_questions_become_system_one() {
    let body = json!({
        "model": "gpt-6-luna",
        "input": [
            {"role": "user", "content": [
                {"type": "input_text", "text": "First line"},
                {"type": "input_text", "text": "Second line"}
            ]},
            {"role": "user", "content": "Last message"}
        ],
        "questions": [
            {"type": "predicate", "name": "urgent", "instructions": "Is it urgent?"},
            {"type": "choice", "name": "department", "instructions": "Which team?",
             "choices": [{"value": "billing", "description": "Payments"}, {"value": true}]},
            {"type": "score", "name": "severity", "instructions": "How bad?",
             "levels": [{"label": "Low"}, {"label": "High", "description": "Blocked"}]}
        ]
    });
    let out = translated(body);
    assert_eq!(out["model"], "upstream-model");
    assert_eq!(out["state"], "First line\nSecond line\n\nLast message");
    assert_eq!(out["questions"]["urgent"]["type"], "noul");
    assert_eq!(out["questions"]["urgent"]["instructions"], "Is it urgent?");
    assert_eq!(out["questions"]["department"]["type"], "choice");
    assert_eq!(
        out["questions"]["department"]["criteria"],
        json!({"billing": "Payments", "true": null})
    );
    assert_eq!(out["questions"]["severity"]["type"], "score");
    assert_eq!(
        out["questions"]["severity"]["criteria"],
        json!(["Low", "High: Blocked"])
    );
}

#[test]
fn unnamed_questions_get_collision_free_keys() {
    let body = json!({
        "input": "hello",
        "questions": [
            {"type": "predicate", "instructions": "?"},
            {"type": "predicate", "name": "question_1", "instructions": "?"}
        ]
    });
    let out = translated(body);
    let keys: Vec<&String> = out["questions"].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["_question_1", "question_1"]);
}

#[test]
fn typed_choice_values_round_trip() {
    let body = json!({
        "input": "hello",
        "questions": [{
            "type": "choice",
            "name": "pick",
            "instructions": "Pick",
            "choices": [{"value": true}, {"value": "yes"}, {"value": "other"}]
        }]
    });
    let decoded = decode_body(&body);
    let (_, map) = encode(DecisionTarget::SystemOne, &decoded, "m").unwrap();
    assert_eq!(map.keys, ["pick"]);
    let upstream = json!({
        "answers": {
            "pick": {
                "type": "choice",
                "choice": "other",
                "probabilities": {"yes": 0.3, "other": 0.7},
                "confidence": 0.8
            }
        },
        "usage": {"input_tokens": 12, "output_tokens": 4}
    });
    let out = response(&upstream, &map, "shown");
    assert_eq!(out["answers"][0]["type"], "choice");
    assert_eq!(out["answers"][0]["name"], "pick");
    assert_eq!(out["answers"][0]["choice"], "other");
    assert_eq!(
        out["answers"][0]["probabilities"],
        json!([
            {"value": true, "probability": 0.0},
            {"value": "yes", "probability": 0.3},
            {"value": "other", "probability": 0.7}
        ])
    );
    assert_eq!(out["answers"][0]["confidence"], 0.8);
    assert_eq!(out["usage"]["total_tokens"], 16);
}

#[test]
fn a_missing_answer_becomes_a_refusal() {
    let body = json!({
        "input": "hello",
        "questions": [{"type": "predicate", "name": "x", "instructions": "?"}]
    });
    let decoded = decode_body(&body);
    let (_, map) = encode(DecisionTarget::SystemOne, &decoded, "m").unwrap();
    let out = response(&json!({"answers": {}}), &map, "shown");
    assert_eq!(out["answers"][0], json!({"type": "refusal", "name": "x"}));
    assert_eq!(out["usage"]["input_tokens"], 0);
    assert_eq!(out["usage"]["output_tokens"], 0);
}

#[test]
fn score_labels_come_from_the_request() {
    let body = json!({
        "input": "hello",
        "questions": [{
            "type": "score",
            "name": "severity",
            "instructions": "?",
            "levels": [{"label": "Low"}, {"label": "High"}]
        }]
    });
    let decoded = decode_body(&body);
    let (_, map) = encode(DecisionTarget::SystemOne, &decoded, "m").unwrap();
    let upstream = json!({
        "answers": {"severity": {
            "type": "score", "score": 1.4,
            "probabilities": {"0": 0.0, "1": 1.0}, "confidence": 0.9
        }}
    });
    let out = response(&upstream, &map, "shown");
    assert_eq!(
        out["answers"][0]["probabilities"],
        json!([
            {"value": 0, "label": "Low", "probability": 0.0},
            {"value": 1, "label": "High", "probability": 1.0}
        ])
    );
}

#[test]
fn system_one_limits_are_refused() {
    let images = json!({
        "input": [{"role": "user", "content": [
            {"type": "input_text", "text": "x"},
            {"type": "input_image", "image_url": "https://example.com/x.png"}
        ]}],
        "questions": [{"type": "predicate", "instructions": "?"}]
    });
    let decoded = decode_body(&images);
    assert!(feasible(DecisionTarget::SystemOne, &decoded).is_err());

    let json_input = json!({
        "input": [{"role": "user", "content": [
            {"type": "input_json", "data": {"ticket": "x"}}
        ]}],
        "questions": [{"type": "predicate", "instructions": "?"}]
    });
    let decoded = decode_body(&json_input);
    assert!(feasible(DecisionTarget::SystemOne, &decoded).is_err());

    let too_many = json!({
        "input": "x",
        "questions": [{
            "type": "score", "instructions": "?",
            "levels": (0..11).map(|i| json!({"label": format!("l{i}")})).collect::<Vec<_>>()
        }]
    });
    let decoded = decode_body(&too_many);
    assert!(feasible(DecisionTarget::SystemOne, &decoded).is_err());

    let collision = json!({
        "input": "x",
        "questions": [{
            "type": "choice", "instructions": "?",
            "choices": [{"value": true}, {"value": "true"}]
        }]
    });
    let decoded = decode_body(&collision);
    assert!(feasible(DecisionTarget::SystemOne, &decoded).is_err());
}

#[test]
fn choice_shadow_names_survive() {
    let body = json!({
        "input": "hello",
        "questions": [{
            "type": "choice", "name": "pick", "instructions": "Pick",
            "choices": [
                {"value": "constructor"}, {"value": "__proto__"}, {"value": "toString"}
            ]
        }]
    });
    let out = translated(body);
    assert_eq!(
        out["questions"]["pick"]["criteria"],
        json!({"constructor": null, "__proto__": null, "toString": null})
    );
}

#[test]
fn decode_keeps_the_ir_shape() {
    let body = json!({
        "input": "hello",
        "questions": [{"type": "predicate", "name": "x", "instructions": "?"}]
    });
    let decoded = decode_body(&body);
    assert!(matches!(decoded.request.input, super::Input::Text(_)));
    let question = decoded.request.questions.first().unwrap();
    assert_eq!(question.name.as_deref(), Some("x"));
    assert!(matches!(question.instructions, Instructions::Text(_)));
    assert!(matches!(question.kind, QuestionKind::Predicate));
    let _ = ChoiceValue::Str("x".into());
}
