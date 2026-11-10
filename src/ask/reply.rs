//! What the user returned: per question, the chosen option IDs and the typed
//! text, or an explicit dismissal.

use super::{Question, Request};
use crate::json::Value;

/// The longest text one answer may carry.
pub const TEXT_LIMIT: usize = 64 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answer {
    /// The question's ID.
    pub id: String,
    pub choices: Vec<String>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Reply {
    Answered(Vec<Answer>),
    Dismissed,
}

impl Answer {
    fn value(&self) -> Value {
        Value::object([
            ("id", Value::string(&self.id)),
            (
                "choices",
                Value::Array(self.choices.iter().map(Value::string).collect()),
            ),
            ("text", Value::string(&self.text)),
        ])
    }

    fn parse(value: &Value) -> Option<Self> {
        Some(Self {
            id: value.get("id")?.as_str()?.into(),
            choices: value
                .get("choices")?
                .as_array()?
                .iter()
                .map(|choice| choice.as_str().map(String::from))
                .collect::<Option<_>>()?,
            text: value.get("text")?.as_str()?.into(),
        })
    }
}

impl Reply {
    pub(crate) fn result(&self) -> Value {
        match self {
            Self::Answered(answers) => Value::object([
                ("outcome", Value::string("answered")),
                (
                    "answers",
                    Value::Array(answers.iter().map(Answer::value).collect()),
                ),
            ]),
            Self::Dismissed => Value::object([("outcome", Value::string("dismissed"))]),
        }
    }

    /// A saved result with one answer per question, or a dismissal.
    pub(crate) fn parse(value: &Value) -> Option<Self> {
        match value.get("outcome")?.as_str()? {
            "answered" => value
                .get("answers")?
                .as_array()?
                .iter()
                .map(Answer::parse)
                .collect::<Option<_>>()
                .map(Self::Answered),
            "dismissed" => Some(Self::Dismissed),
            _ => None,
        }
    }
}

impl Request {
    /// Every question answered once, in order, with its own options: one
    /// choice at most for a single choice, and a choice or words for each.
    pub(crate) fn check(&self, answers: &[Answer]) -> Result<(), String> {
        if answers.len() != self.questions.len() {
            return Err("Answer every question".into());
        }
        for (question, answer) in self.questions.iter().zip(answers) {
            question.check_answer(answer)?;
        }
        Ok(())
    }
}

impl Question {
    pub(crate) fn check_answer(&self, answer: &Answer) -> Result<(), String> {
        if answer.id != self.id {
            return Err("Answers do not match the questions".into());
        }
        if answer.text.len() > TEXT_LIMIT {
            return Err("Answer exceeds 64 KiB".into());
        }
        if answer.choices.is_empty() && answer.text.trim().is_empty() {
            return Err(format!("Answer {}", self.title));
        }
        if !self.is_multiple() && answer.choices.len() > 1 {
            return Err("Choose one answer".into());
        }
        for (index, choice) in answer.choices.iter().enumerate() {
            if !self.options.iter().any(|option| &option.id == choice)
                || answer.choices[..index].contains(choice)
            {
                return Err("Unknown answer choice".into());
            }
        }
        Ok(())
    }
}
