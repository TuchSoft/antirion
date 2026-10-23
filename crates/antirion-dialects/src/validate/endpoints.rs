//! Completions, Embeddings, image generation, speech, Responses and Anthropic Messages.

use antirion_json::Skim;
use serde_json::Value;

use super::{
    Check, Table, ValidateOpts, Violation, array, bad_value, boolean, index, integer_in, key, member_value,
    missing, non_empty_string, number_in, object, string, wrong_type,
};

pub(crate) static COMPLETIONS: Table = Table {
    member: completions,
    required: &["prompt"],
    finish: None,
    on_error: None,
};

pub(crate) static EMBEDDINGS: Table = Table {
    member: embeddings,
    required: &["input"],
    finish: None,
    on_error: None,
};

pub(crate) static IMAGES: Table = Table {
    member: images,
    required: &["prompt"],
    finish: None,
    on_error: None,
};

pub(crate) static SPEECH: Table = Table {
    member: speech,
    required: &["input", "voice"],
    finish: None,
    on_error: None,
};

pub(crate) static MODERATIONS: Table = Table {
    member: moderations,
    required: &["input"],
    finish: None,
    on_error: None,
};

pub(crate) static RESPONSES: Table = Table {
    member: responses,
    required: &[],
    finish: Some(responses_source),
    on_error: None,
};

pub(crate) static DECISIONS: Table = Table {
    member: decisions,
    required: &["input", "questions"],
    finish: None,
    on_error: None,
};

pub(crate) static SYSTEMONE: Table = Table {
    member: systemone,
    required: &["state", "questions"],
    finish: None,
    on_error: None,
};

pub(crate) static MESSAGES: Table = Table {
    member: messages_member,
    required: &["max_tokens", "messages"],
    finish: None,
    on_error: Some(openai_hint),
};

/// `count_tokens` takes a Messages body without `max_tokens`.
pub(crate) static COUNT_TOKENS: Table = Table {
    member: messages_member,
    required: &["messages"],
    finish: None,
    on_error: Some(openai_hint),
};

fn completions(name: &str, v: &Value, _opts: &ValidateOpts) -> Check {
    match name {
        "prompt" => prompt(v),
        "stream" if !v.is_null() => boolean(v, name),
        _ => Ok(()),
    }
}

/// A string, an array of strings, an array of integers, or an array of integer arrays.
fn prompt(v: &Value) -> Check {
    let expected = "a string, an array of strings, an array of integers or an array of integer arrays";
    match v {
        Value::String(_) => Ok(()),
        Value::Array(items) => match items.first() {
            None | Some(Value::String(_)) => each(items, "prompt", |s, p| string(s, p).map(|_| ())),
            Some(Value::Array(_)) => each(items, "prompt", |t, p| tokens(t, p, false)),
            Some(_) => each(items, "prompt", token),
        },
        _ => Err(wrong_type("prompt", expected)),
    }
}

fn embeddings(name: &str, v: &Value, _opts: &ValidateOpts) -> Check {
    if name != "input" {
        return Ok(());
    }
    match v {
        Value::String(_) => non_empty_string(v, "input"),
        Value::Array(items) => {
            if items.is_empty() {
                return Err(bad_value("input", "must not be empty."));
            }
            match items.first() {
                Some(Value::String(_)) => each(items, "input", non_empty_string),
                Some(Value::Array(_)) => each(items, "input", |t, p| tokens(t, p, true)),
                _ => each(items, "input", token),
            }
        }
        _ => Err(wrong_type(
            "input",
            "a string, an array of strings, an array of integers or an array of integer arrays",
        )),
    }
}

fn each(items: &[Value], parent: &str, f: impl Fn(&Value, &str) -> Check) -> Check {
    for (i, item) in items.iter().enumerate() {
        f(item, &index(parent, i))?;
    }
    Ok(())
}

fn token(v: &Value, path: &str) -> Check {
    integer_in(v, path, i64::MIN, None)
}

fn tokens(v: &Value, path: &str, non_empty: bool) -> Check {
    let list = array(v, path)?;
    if non_empty && list.is_empty() {
        return Err(bad_value(path, "must not be empty."));
    }
    each(list, path, token)
}

fn images(name: &str, v: &Value, opts: &ValidateOpts) -> Check {
    match name {
        "prompt" => non_empty_string(v, name),
        "n" if !v.is_null() => {
            let max = i64::try_from(opts.images_max_n).unwrap_or(i64::MAX);
            integer_in(v, name, 1, Some(max))
        }
        _ => Ok(()),
    }
}

fn moderations(name: &str, v: &Value, _opts: &ValidateOpts) -> Check {
    match name {
        "input" => match v {
            Value::String(_) | Value::Array(_) => Ok(()),
            _ => Err(wrong_type(name, "a string or an array")),
        },
        _ => Ok(()),
    }
}

fn decisions(name: &str, v: &Value, _opts: &ValidateOpts) -> Check {
    match name {
        "input" => decisions_input(v),
        "questions" => decisions_questions(v),
        "stream" if !v.is_null() => must_be_false(name, v),
        _ => Ok(()),
    }
}

fn must_be_false(name: &str, v: &Value) -> Check {
    match v {
        Value::Bool(false) => Ok(()),
        Value::Bool(true) => Err(bad_value(name, "must be false: this endpoint does not stream.")),
        _ => Err(wrong_type(name, "a boolean")),
    }
}

fn decisions_input(v: &Value) -> Check {
    match v {
        Value::String(_) => Ok(()),
        Value::Array(items) => {
            if items.is_empty() {
                return Err(bad_value("input", "must not be empty."));
            }
            for (i, item) in items.iter().enumerate() {
                decisions_item(item, &index("input", i))?;
            }
            Ok(())
        }
        _ => Err(wrong_type("input", "a string or an array of input items")),
    }
}

fn decisions_item(v: &Value, path: &str) -> Check {
    let item = object(v, path)?;
    if let Some(role) = item.get("role") {
        let role_path = key(path, "role");
        match string(role, &role_path)? {
            "user" | "developer" | "system" | "assistant" => {}
            _ => {
                return Err(bad_value(
                    &role_path,
                    "must be user, developer, system or assistant.",
                ));
            }
        }
        let content_path = key(path, "content");
        return match item.get("content") {
            None => Err(missing(&content_path)),
            Some(Value::String(_)) => Ok(()),
            Some(Value::Array(parts)) => content_parts(parts, &content_path),
            Some(_) => Err(wrong_type(&content_path, "a string or an array of content parts")),
        };
    }
    // Tool items (`function_call`, `function_call_output`, `custom_tool_call`, ...) are accepted
    // here and refused by the System One translation when they cannot be carried.
    match item.get("type") {
        Some(Value::String(_)) => Ok(()),
        Some(_) => Err(wrong_type(&key(path, "type"), "a string")),
        None => Err(missing(&key(path, "role"))),
    }
}

fn content_parts(parts: &[Value], path: &str) -> Check {
    if parts.is_empty() {
        return Err(bad_value(path, "must not be empty."));
    }
    for (i, part) in parts.iter().enumerate() {
        let part_path = index(path, i);
        let p = object(part, &part_path)?;
        let type_path = key(&part_path, "type");
        let kind = match p.get("type") {
            None => return Err(missing(&type_path)),
            Some(t) => string(t, &type_path)?,
        };
        match kind {
            "input_text" => {
                let text_path = key(&part_path, "text");
                match p.get("text") {
                    None => return Err(missing(&text_path)),
                    Some(t) => {
                        string(t, &text_path)?;
                    }
                }
            }
            "input_image" => {
                let url_path = key(&part_path, "image_url");
                match p.get("image_url") {
                    None => return Err(missing(&url_path)),
                    Some(u) => {
                        string(u, &url_path)?;
                    }
                }
            }
            "input_json" => match p.get("data") {
                Some(Value::Object(_) | Value::Array(_)) => {}
                Some(_) => return Err(wrong_type(&key(&part_path, "data"), "an object or an array")),
                None => return Err(missing(&key(&part_path, "data"))),
            },
            _ => {
                return Err(bad_value(
                    &type_path,
                    "must be input_text, input_image or input_json.",
                ));
            }
        }
    }
    Ok(())
}

fn decisions_questions(v: &Value) -> Check {
    let questions = array(v, "questions")?;
    if questions.is_empty() {
        return Err(bad_value("questions", "must contain at least one question."));
    }
    if questions.len() > 400 {
        return Err(bad_value("questions", "must contain at most 400 questions."));
    }
    let mut names: Vec<&str> = Vec::new();
    for (i, question) in questions.iter().enumerate() {
        let path = index("questions", i);
        let q = object(question, &path)?;
        let type_path = key(&path, "type");
        let kind = match q.get("type") {
            None => return Err(missing(&type_path)),
            Some(t) => string(t, &type_path)?,
        };
        if !matches!(kind, "predicate" | "choice" | "score") {
            return Err(bad_value(&type_path, "must be predicate, choice or score."));
        }
        if let Some(name) = q.get("name") {
            let name = string(name, &key(&path, "name"))?;
            if names.contains(&name) {
                return Err(bad_value(&key(&path, "name"), "must be unique."));
            }
            names.push(name);
        }
        match q.get("instructions") {
            None => return Err(missing(&key(&path, "instructions"))),
            Some(v) => instructions(v, &key(&path, "instructions"))?,
        }
        match kind {
            "choice" => choices(q.get("choices"), &key(&path, "choices"))?,
            "score" => levels(q.get("levels"), &key(&path, "levels"))?,
            _ => {}
        }
    }
    Ok(())
}

fn instructions(v: &Value, path: &str) -> Check {
    match v {
        Value::String(s) if s.is_empty() => Err(bad_value(path, "must not be empty.")),
        Value::String(_) => Ok(()),
        Value::Array(parts) => content_parts(parts, path),
        _ => Err(wrong_type(path, "a string or an array of content parts")),
    }
}

fn choices(v: Option<&Value>, path: &str) -> Check {
    let Some(v) = v else {
        return Err(missing(path));
    };
    let list = array(v, path)?;
    if list.len() < 2 || list.len() > 255 {
        return Err(bad_value(path, "must contain between 2 and 255 choices."));
    }
    let mut values: Vec<&Value> = Vec::new();
    for (i, choice) in list.iter().enumerate() {
        let choice_path = index(path, i);
        let c = object(choice, &choice_path)?;
        let value_path = key(&choice_path, "value");
        let value = match c.get("value") {
            None => return Err(missing(&value_path)),
            Some(v @ (Value::String(_) | Value::Bool(_))) => v,
            Some(_) => return Err(wrong_type(&value_path, "a string or a boolean")),
        };
        if values.contains(&value) {
            return Err(bad_value(&value_path, "must be unique."));
        }
        values.push(value);
        if let Some(description) = c.get("description") {
            string(description, &key(&choice_path, "description"))?;
        }
    }
    Ok(())
}

fn levels(v: Option<&Value>, path: &str) -> Check {
    let Some(v) = v else {
        return Err(missing(path));
    };
    let list = array(v, path)?;
    if list.len() < 2 {
        return Err(bad_value(path, "must contain at least two levels."));
    }
    for (i, level) in list.iter().enumerate() {
        let level_path = index(path, i);
        let l = object(level, &level_path)?;
        let label_path = key(&level_path, "label");
        match l.get("label") {
            None => return Err(missing(&label_path)),
            Some(label) => {
                string(label, &label_path)?;
            }
        }
        if let Some(description) = l.get("description") {
            string(description, &key(&level_path, "description"))?;
        }
    }
    Ok(())
}

fn systemone(name: &str, v: &Value, _opts: &ValidateOpts) -> Check {
    match name {
        "state" => structured_text(v, name),
        "questions" => systemone_questions(v),
        "assets" if !v.is_null() => object(v, name).map(|_| ()),
        "stream" if !v.is_null() => must_be_false(name, v),
        _ => Ok(()),
    }
}

fn systemone_questions(v: &Value) -> Check {
    let questions = object(v, "questions")?;
    if questions.is_empty() {
        return Err(bad_value("questions", "must contain at least one question."));
    }
    for (name, question) in questions {
        let path = key("questions", name);
        let q = match question {
            Value::Object(q) => q,
            _ => return Err(wrong_type(&path, "an object")),
        };
        let type_path = key(&path, "type");
        let kind = match q.get("type") {
            None => return Err(missing(&type_path)),
            Some(t) => string(t, &type_path)?,
        };
        if !matches!(kind, "noul" | "choice" | "score" | "box") {
            return Err(bad_value(&type_path, "must be noul, choice, score or box."));
        }
        match q.get("instructions") {
            None => return Err(missing(&key(&path, "instructions"))),
            Some(v) => structured_text(v, &key(&path, "instructions"))?,
        }
        match kind {
            "noul" => {
                if let Some(c) = q.get("criteria") {
                    let c_path = key(&path, "criteria");
                    let c = object(c, &c_path)?;
                    for side in ["true", "false"] {
                        if let Some(v) = c.get(side) {
                            structured_text(v, &key(&c_path, side))?;
                        }
                    }
                }
            }
            "choice" => {
                let criteria = q
                    .get("criteria")
                    .ok_or_else(|| missing(&key(&path, "criteria")))?;
                let c_path = key(&path, "criteria");
                let c = object(criteria, &c_path)?;
                if c.len() < 2 || c.len() > 255 {
                    return Err(bad_value(&c_path, "must contain between 2 and 255 options."));
                }
                for (option, description) in c {
                    if !description.is_null() {
                        structured_text(description, &key(&c_path, option))?;
                    }
                }
            }
            "score" => {
                let criteria = q
                    .get("criteria")
                    .ok_or_else(|| missing(&key(&path, "criteria")))?;
                let c_path = key(&path, "criteria");
                let c = array(criteria, &c_path)?;
                if c.len() < 2 || c.len() > 10 {
                    return Err(bad_value(&c_path, "must contain between 2 and 10 levels."));
                }
                for (i, level) in c.iter().enumerate() {
                    structured_text(level, &index(&c_path, i))?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn structured_text(v: &Value, path: &str) -> Check {
    match v {
        Value::String(_) | Value::Object(_) | Value::Array(_) => Ok(()),
        _ => Err(wrong_type(path, "a string, an object or an array")),
    }
}

fn speech(name: &str, v: &Value, _opts: &ValidateOpts) -> Check {
    match name {
        "input" => non_empty_string(v, name),
        "voice" => match v {
            Value::String(_) => non_empty_string(v, name),
            Value::Object(_) => Ok(()),
            _ => Err(wrong_type(name, "a string or an object")),
        },
        _ => Ok(()),
    }
}

fn responses(name: &str, v: &Value, _opts: &ValidateOpts) -> Check {
    match name {
        "input" if !v.is_null() => match v {
            Value::String(_) | Value::Array(_) => Ok(()),
            _ => Err(wrong_type(name, "a string or an array of items")),
        },
        "stream" if !v.is_null() => boolean(v, name),
        _ => Ok(()),
    }
}

/// A Responses request needs something to answer: `input`, or a stored conversation to continue.
fn responses_source(skim: &Skim) -> Check {
    if ["input", "previous_response_id", "conversation"]
        .iter()
        .any(|m| skim.member(m).is_some())
    {
        return Ok(());
    }
    Err(missing("input"))
}

fn messages_member(name: &str, v: &Value, _opts: &ValidateOpts) -> Check {
    match name {
        "max_tokens" => integer_in(v, name, 0, None),
        "messages" => anthropic_messages(v),
        "temperature" if !v.is_null() => number_in(v, name, 0.0, 1.0),
        "top_p" if !v.is_null() => number_in(v, name, 0.0, 1.0),
        "stream" if !v.is_null() => boolean(v, name),
        "thinking" if !v.is_null() => thinking(v),
        _ => Ok(()),
    }
}

fn anthropic_messages(v: &Value) -> Check {
    let list = array(v, "messages")?;
    if list.is_empty() {
        return Err(bad_value("messages", "must contain at least one message."));
    }
    for (i, m) in list.iter().enumerate() {
        let path = index("messages", i);
        let msg = object(m, &path)?;
        let role_path = key(&path, "role");
        match msg.get("role") {
            None => return Err(missing(&role_path)),
            Some(r) => {
                if !["user", "assistant", "system"].contains(&string(r, &role_path)?) {
                    return Err(bad_value(&role_path, "must be one of user, assistant, system."));
                }
            }
        }
        let content_path = key(&path, "content");
        match msg.get("content") {
            None => return Err(missing(&content_path)),
            Some(Value::String(_)) => {}
            Some(Value::Array(blocks)) => {
                for (j, b) in blocks.iter().enumerate() {
                    let block_path = index(&content_path, j);
                    let block = object(b, &block_path)?;
                    let type_path = key(&block_path, "type");
                    match block.get("type") {
                        None => return Err(missing(&type_path)),
                        Some(t) => {
                            string(t, &type_path)?;
                        }
                    }
                }
            }
            Some(_) => {
                return Err(wrong_type(
                    &content_path,
                    "a string or an array of content blocks",
                ));
            }
        }
    }
    Ok(())
}

fn thinking(v: &Value) -> Check {
    let t = object(v, "thinking")?;
    if t.get("type").and_then(Value::as_str) != Some("enabled") {
        return Ok(());
    }
    match t.get("budget_tokens") {
        None => Err(missing("thinking.budget_tokens")),
        Some(b) => integer_in(b, "thinking.budget_tokens", 1024, None),
    }
}

/// Bodies shaped for Chat Completions sent to Messages get a pointer to the right endpoint.
fn openai_hint(v: Violation, raw: &[u8], skim: &Skim) -> Violation {
    let system_first = skim
        .member("messages")
        .and_then(|m| member_value(raw, &m.value))
        .and_then(|msgs| {
            msgs.as_array()
                .and_then(|a| a.first())
                .and_then(|m| m.get("role"))
                .and_then(Value::as_str)
                .map(|r| r == "system")
        })
        .unwrap_or(false);
    if skim.member("max_completion_tokens").is_none() && !system_first {
        return v;
    }
    Violation {
        message: format!(
            "{} Hint: this looks like an OpenAI Chat Completions body; send it to /v1/chat/completions.",
            v.message
        ),
        ..v
    }
}

#[cfg(test)]
mod tests {
    use super::super::ValidateOpts;
    use super::super::ViolationKind::{Missing, Type, Value as Val};
    use super::super::tests::{check_at, rejects};
    use crate::endpoints::{Matched, match_endpoint};

    const EMB: &str = "/v1/embeddings";
    const IMG: &str = "/v1/images/generations";
    const SPEECH: &str = "/v1/audio/speech";
    const RESP: &str = "/v1/responses";
    const MSG: &str = "/v1/messages";
    const DEC: &str = "/v1/decisions";
    const SYS: &str = "/v1/systemone";

    #[test]
    fn embeddings_inputs() {
        for ok in [r#""hi""#, r#"["a","b"]"#, "[1,2,3]", "[[1,2],[3]]"] {
            let b = format!(r#"{{"model":"m","input":{ok},"dimensions":256,"encoding_format":"base64"}}"#);
            assert_eq!(check_at(EMB, &b), Ok(()), "{b}");
        }
        for (input, param) in [
            ("[]", "input"),
            (r#""""#, "input"),
            (r#"["a",""]"#, "input[1]"),
            ("[[]]", "input[0]"),
        ] {
            rejects(
                EMB,
                &format!(r#"{{"model":"m","input":{input}}}"#),
                param,
                Some(Val),
            );
        }
        rejects(EMB, r#"{"model":"m"}"#, "input", Some(Missing));
        rejects(EMB, r#"{"model":"m","input":5}"#, "input", Some(Type));
        rejects(EMB, r#"{"model":"m","input":["a",5]}"#, "input[1]", Some(Type));
        rejects(
            EMB,
            r#"{"model":"m","input":[[1],["x"]]}"#,
            "input[1][0]",
            Some(Type),
        );
    }

    #[test]
    fn images() {
        rejects(IMG, "{}", "prompt", Some(Missing));
        rejects(IMG, r#"{"model":"m"}"#, "prompt", Some(Missing));
        rejects(IMG, r#"{"model":"m","prompt":""}"#, "prompt", Some(Val));
        rejects(IMG, r#"{"model":"m","prompt":5}"#, "prompt", Some(Type));
        for (n, kind) in [
            ("0", Some(Val)),
            ("11", Some(Val)),
            ("2.5", None),
            (r#""2""#, None),
        ] {
            rejects(
                IMG,
                &format!(r#"{{"model":"m","prompt":"a cat","n":{n}}}"#),
                "n",
                kind,
            );
        }
        for n in ["1", "10"] {
            assert_eq!(
                check_at(IMG, &format!(r#"{{"model":"m","prompt":"a cat","n":{n}}}"#)),
                Ok(())
            );
        }
    }

    #[test]
    fn images_n_bound_follows_the_option() {
        let Matched::Found(e) = match_endpoint(&http::Method::POST, IMG) else {
            panic!()
        };
        let raw = br#"{"model":"m","prompt":"a cat","n":3}"#;
        let s = antirion_json::skim(raw, &antirion_json::JsonLimits::default()).unwrap();
        let v = super::super::validate_json(
            &e,
            raw,
            &s,
            None,
            &ValidateOpts {
                images_max_n: 2,
                templated: false,
            },
        )
        .unwrap_err();
        assert_eq!((v.path.as_str(), v.kind), ("n", Val));
        let raw = br#"{"model":"m","prompt":"a cat","n":2}"#;
        let s = antirion_json::skim(raw, &antirion_json::JsonLimits::default()).unwrap();
        assert!(
            super::super::validate_json(
                &e,
                raw,
                &s,
                None,
                &ValidateOpts {
                    images_max_n: 2,
                    templated: false
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn speech() {
        rejects(SPEECH, r#"{"model":"m","voice":"alloy"}"#, "input", Some(Missing));
        rejects(SPEECH, r#"{"model":"m","input":"hello"}"#, "voice", Some(Missing));
        rejects(
            SPEECH,
            r#"{"model":"m","input":"hello","voice":5}"#,
            "voice",
            Some(Type),
        );
        assert_eq!(check_at(SPEECH, r#"{"input":"hello","voice":"alloy"}"#), Ok(()));
        assert_eq!(
            check_at(SPEECH, r#"{"model":"m","input":"hello","voice":{"id":"v1"}}"#),
            Ok(())
        );
    }

    #[test]
    fn completions_require_a_prompt() {
        rejects(
            "/v1/completions",
            r#"{"model":"m","messages":[{"role":"user","content":"hi"}]}"#,
            "prompt",
            Some(Missing),
        );
        rejects("/v1/completions", "{}", "prompt", Some(Missing));
        rejects("/v1/completions", r#"{"prompt":{}}"#, "prompt", Some(Type));
        rejects(
            "/v1/completions",
            r#"{"prompt":["a",1]}"#,
            "prompt[1]",
            Some(Type),
        );
        for ok in [r#""x""#, r#"["a","b"]"#, "[1,2]", "[[1],[2,3]]", "[]"] {
            assert_eq!(
                check_at("/v1/completions", &format!(r#"{{"prompt":{ok}}}"#)),
                Ok(()),
                "{ok}"
            );
        }
    }

    #[test]
    fn responses_need_something_to_answer() {
        rejects(
            RESP,
            r#"{"model":"m","instructions":"hi"}"#,
            "input",
            Some(Missing),
        );
        rejects(RESP, r#"{"model":"m","input":5}"#, "input", Some(Type));
        for ok in [
            r#"{"model":"m","input":"hi"}"#,
            r#"{"input":"hi"}"#,
            r#"{"model":"m","input":[{"type":"vendor_item","x":1},{"role":"user","content":"hi"}]}"#,
            r#"{"model":"m","previous_response_id":"resp_1"}"#,
            r#"{"model":"m","conversation":"conv_1"}"#,
        ] {
            assert_eq!(check_at(RESP, ok), Ok(()), "{ok}");
        }
    }

    #[test]
    fn anthropic_messages() {
        let body = r#"{"model":"m","max_tokens":8,"messages":[{"role":"user","content":"a"},{"role":"robot","content":"b"}]}"#;
        rejects(MSG, body, "messages[1].role", Some(Val));
        rejects(
            MSG,
            r#"{"model":"m","messages":[{"role":"user","content":"a"}]}"#,
            "max_tokens",
            Some(Missing),
        );
        rejects(
            MSG,
            r#"{"max_tokens":-1,"messages":[{"role":"user","content":"a"}]}"#,
            "max_tokens",
            Some(Val),
        );
        rejects(
            MSG,
            r#"{"max_tokens":1.5,"messages":[{"role":"user","content":"a"}]}"#,
            "max_tokens",
            Some(Type),
        );
        rejects(MSG, r#"{"max_tokens":1,"messages":[]}"#, "messages", Some(Val));
        rejects(MSG, r#"{"max_tokens":1,"messages":"x"}"#, "messages", Some(Type));
        rejects(
            MSG,
            r#"{"max_tokens":1,"messages":[{"role":"user","content":[{"text":"a"}]}]}"#,
            "messages[0].content[0].type",
            Some(Missing),
        );
        rejects(
            MSG,
            r#"{"max_tokens":1,"temperature":1.5,"messages":[{"role":"user","content":"a"}]}"#,
            "temperature",
            Some(Val),
        );
        rejects(
            MSG,
            r#"{"max_tokens":1,"thinking":{"type":"enabled","budget_tokens":1000},"messages":[{"role":"user","content":"a"}]}"#,
            "thinking.budget_tokens",
            Some(Val),
        );
        for ok in [
            r#"{"model":"m","max_tokens":0,"messages":[{"role":"user","content":"a"}]}"#,
            r#"{"max_tokens":8,"messages":[{"role":"user","content":"a"},{"role":"system","content":"s"}]}"#,
            r#"{"max_tokens":8,"thinking":{"type":"enabled","budget_tokens":1024},"messages":[{"role":"user","content":[{"type":"text","text":"a"}]}]}"#,
        ] {
            assert_eq!(check_at(MSG, ok), Ok(()), "{ok}");
        }
    }

    #[test]
    fn decisions_bodies() {
        let ok = r#"{"model":"gpt-6-luna","input":"I was charged twice.",
            "questions":[
              {"type":"predicate","instructions":"Is this billing?"},
              {"type":"choice","name":"department","instructions":"Which?",
               "choices":[{"value":"billing"},{"value":true}]},
              {"type":"score","name":"severity","instructions":"How bad?",
               "levels":[{"label":"Low"},{"label":"High"}]}]}"#;
        assert_eq!(check_at(DEC, ok), Ok(()));
        assert_eq!(
            check_at(
                DEC,
                r#"{"input":[{"role":"user","content":[{"type":"input_text","text":"hi"},{"type":"input_image","image_url":"https://x/y.png"}]}],"questions":[{"type":"predicate","instructions":"?"}]}"#
            ),
            Ok(())
        );
        rejects(DEC, r#"{"model":"m"}"#, "input", Some(Missing));
        rejects(DEC, r#"{"input":"x"}"#, "questions", Some(Missing));
        rejects(
            DEC,
            r#"{"input":[],"questions":[{"type":"predicate","instructions":"?"}]}"#,
            "input",
            Some(Val),
        );
        rejects(DEC, r#"{"input":"x","questions":[]}"#, "questions", Some(Val));
        rejects(
            DEC,
            r#"{"input":"x","questions":[{"type":"predicate","instructions":"?"},{"type":"predicate","name":"a","instructions":"?"},{"type":"predicate","name":"a","instructions":"?"}]}"#,
            "questions[2].name",
            Some(Val),
        );
        rejects(
            DEC,
            r#"{"input":"x","questions":[{"type":"choice","instructions":"?","choices":[{"value":"a"}]}]}"#,
            "questions[0].choices",
            Some(Val),
        );
        rejects(
            DEC,
            r#"{"input":"x","questions":[{"type":"choice","instructions":"?","choices":[{"value":"a"},{"value":"a"}]}]}"#,
            "questions[0].choices[1].value",
            Some(Val),
        );
        rejects(
            DEC,
            r#"{"input":"x","questions":[{"type":"score","instructions":"?","levels":[{"label":"a"}]}]}"#,
            "questions[0].levels",
            Some(Val),
        );
        rejects(
            DEC,
            r#"{"input":"x","stream":true,"questions":[{"type":"predicate","instructions":"?"}]}"#,
            "stream",
            Some(Val),
        );
    }

    #[test]
    fn systemone_bodies() {
        let ok = r#"{"model":"jev-1.13.0","state":"I was charged twice.",
            "questions":{
              "billing":{"type":"noul","instructions":"Is this billing?"},
              "team":{"type":"choice","instructions":"Which?",
                      "criteria":{"billing":"Payments","technical":null}},
              "severity":{"type":"score","instructions":"How bad?",
                           "criteria":["Low",{"what":"High"}]}}}"#;
        assert_eq!(check_at(SYS, ok), Ok(()));
        assert_eq!(
            check_at(
                SYS,
                r#"{"state":{"ticket":{"body":"x"}},"questions":{"q":{"type":"noul","instructions":"?"}}}"#
            ),
            Ok(())
        );
        rejects(SYS, r#"{"model":"m"}"#, "state", Some(Missing));
        rejects(SYS, r#"{"state":"x"}"#, "questions", Some(Missing));
        rejects(SYS, r#"{"state":"x","questions":{}}"#, "questions", Some(Val));
        rejects(
            SYS,
            r#"{"state":"x","questions":{"q":{"type":"choice","instructions":"?","criteria":{"a":"b"}}}}"#,
            "questions.q.criteria",
            Some(Val),
        );
        rejects(
            SYS,
            r#"{"state":"x","questions":{"q":{"type":"score","instructions":"?","criteria":["a"]}}}"#,
            "questions.q.criteria",
            Some(Val),
        );
        rejects(
            SYS,
            r#"{"state":"x","stream":true,"questions":{"q":{"type":"noul","instructions":"?"}}}"#,
            "stream",
            Some(Val),
        );
    }

    #[test]
    fn openai_shaped_messages_bodies_get_a_hint() {
        let v = check_at(
            MSG,
            r#"{"model":"m","max_completion_tokens":8,"messages":[{"role":"user","content":"a"}]}"#,
        )
        .unwrap_err();
        assert_eq!(v.path, "max_tokens");
        assert!(v.message.starts_with("max_tokens"));
        assert!(v.message.contains("/v1/chat/completions"));
        let v = check_at(
            MSG,
            r#"{"max_tokens":1,"messages":[{"role":"user","content":"a"},{"role":"x","content":"b"}]}"#,
        )
        .unwrap_err();
        assert!(!v.message.contains("/v1/chat/completions"));
    }
}
