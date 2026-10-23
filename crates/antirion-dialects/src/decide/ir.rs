//! The decision IR: the request shape both wire formats are decoded into, the answers they decode
//! back from, and the per-attempt mapping needed to restore client field names and typed values.

use serde_json::Value;

/// The upstream format a translated decision request is encoded for.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DecisionTarget {
    /// TypeSafe's System One (Jev, Microsoft-Decision-1).
    SystemOne,
}

/// The shared evidence of a request.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// One text.
    Text(String),
    /// OpenAI input items (messages, function calls, outputs).
    Messages(Vec<Value>),
    /// System One structured state.
    State(Value),
}

/// What a question asks.
#[derive(Clone, Debug, PartialEq)]
pub enum Instructions {
    Text(String),
    /// OpenAI content parts.
    Parts(Vec<Value>),
}

/// A choice value as the client typed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChoiceValue {
    Str(String),
    Bool(bool),
}

impl ChoiceValue {
    /// The value as the System One option key.
    #[must_use]
    pub fn key(&self) -> String {
        match self {
            ChoiceValue::Str(s) => s.clone(),
            ChoiceValue::Bool(b) => b.to_string(),
        }
    }

    /// The value as the client sent it.
    #[must_use]
    pub fn to_json(&self) -> Value {
        match self {
            ChoiceValue::Str(s) => Value::String(s.clone()),
            ChoiceValue::Bool(b) => Value::Bool(*b),
        }
    }

    /// A string or boolean JSON value.
    #[must_use]
    pub fn from_json(v: &Value) -> Option<ChoiceValue> {
        match v {
            Value::String(s) => Some(ChoiceValue::Str(s.clone())),
            Value::Bool(b) => Some(ChoiceValue::Bool(*b)),
            _ => None,
        }
    }
}

/// One choice option.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub value: ChoiceValue,
    pub description: Option<String>,
}

/// One score level.
#[derive(Clone, Debug, PartialEq)]
pub struct Level {
    pub label: String,
    pub description: Option<String>,
}

/// A question's type and its options.
#[derive(Clone, Debug, PartialEq)]
pub enum QuestionKind {
    Predicate,
    Choice(Vec<Choice>),
    Score(Vec<Level>),
}

/// One question.
#[derive(Clone, Debug, PartialEq)]
pub struct Question {
    pub name: Option<String>,
    pub instructions: Instructions,
    pub kind: QuestionKind,
}

/// A decoded request.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub input: Input,
    pub questions: Vec<Question>,
}

/// A decoded client request, shared by every attempt of the request.
#[derive(Clone, Debug)]
pub struct Decoded {
    pub request: Request,
}

/// What one translated attempt needs to map its answer back.
#[derive(Clone, Debug)]
pub struct DecideMap {
    /// The System One key of each question, in request order.
    pub keys: Vec<String>,
    /// The client's questions, in request order.
    pub questions: Vec<Question>,
}

/// The upstream's token usage.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
}
