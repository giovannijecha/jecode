//! Answers in progress, saved with the session input apart from the main
//! draft. An earlier draft held one text, caret and choice.

use super::{MAX_OPTIONS, MAX_QUESTIONS, TEXT_LIMIT};
use crate::{json::Value, redact::Redactor};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    /// The call's ID.
    pub id: String,
    /// The question in front.
    pub step: usize,
    pub answers: Vec<AnswerDraft>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AnswerDraft {
    /// Selected option IDs.
    pub choices: Vec<String>,
    /// `Other` is selected.
    pub other: bool,
    pub text: String,
    pub caret: usize,
    /// The cursor's answer row.
    pub row: usize,
}

impl Draft {
    pub(crate) fn value(&self) -> Value {
        Value::object([
            ("id", Value::string(&self.id)),
            ("step", Value::number(self.step)),
            (
                "answers",
                Value::Array(self.answers.iter().map(AnswerDraft::value).collect()),
            ),
        ])
    }

    pub(crate) fn parse(value: &Value) -> Result<Self, String> {
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or("Invalid question draft ID")?;
        let Some(answers) = value.get("answers") else {
            return Ok(Self {
                id: id.into(),
                step: 0,
                answers: vec![AnswerDraft::legacy(value)?],
            });
        };
        let answers = answers
            .as_array()
            .filter(|answers| answers.len() <= MAX_QUESTIONS)
            .ok_or("Invalid question draft answers")?
            .iter()
            .map(AnswerDraft::parse)
            .collect::<Result<Vec<_>, _>>()?;
        let step = value
            .get("step")
            .and_then(Value::as_usize)
            .filter(|step| *step < answers.len().max(1))
            .ok_or("Invalid question draft step")?;
        Ok(Self {
            id: id.into(),
            step,
            answers,
        })
    }

    /// Known credentials masked in every saved text.
    pub(crate) fn redacted(&self, redactor: &Redactor) -> Self {
        Self {
            id: redactor.text(&self.id),
            step: self.step,
            answers: self
                .answers
                .iter()
                .map(|answer| {
                    let (text, caret) = redactor.text_cursor(&answer.text, answer.caret);
                    AnswerDraft {
                        choices: answer
                            .choices
                            .iter()
                            .map(|choice| redactor.text(choice))
                            .collect(),
                        other: answer.other,
                        caret: if text.is_char_boundary(caret) {
                            caret
                        } else {
                            text.len()
                        },
                        text,
                        row: answer.row,
                    }
                })
                .collect(),
        }
    }
}

impl AnswerDraft {
    fn value(&self) -> Value {
        Value::object([
            (
                "choices",
                Value::Array(self.choices.iter().map(Value::string).collect()),
            ),
            ("other", Value::Bool(self.other)),
            ("text", Value::string(&self.text)),
            ("caret", Value::number(self.caret)),
            ("row", Value::number(self.row)),
        ])
    }

    fn parse(value: &Value) -> Result<Self, String> {
        let choices = value
            .get("choices")
            .and_then(Value::as_array)
            .filter(|choices| choices.len() <= MAX_OPTIONS)
            .and_then(|choices| {
                choices
                    .iter()
                    .map(|choice| choice.as_str().map(String::from))
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or("Invalid question draft choices")?;
        let other = match value.get("other") {
            Some(Value::Bool(other)) => *other,
            _ => return Err("Invalid question draft choice".into()),
        };
        let row = value
            .get("row")
            .and_then(Value::as_usize)
            .filter(|row| *row <= MAX_OPTIONS)
            .ok_or("Invalid question draft row")?;
        let (text, caret) = text(value, "caret")?;
        Ok(Self {
            choices,
            other,
            text,
            caret,
            row,
        })
    }

    /// `{id, text, cursor, choice}` from before several questions.
    fn legacy(value: &Value) -> Result<Self, String> {
        let (text, caret) = text(value, "cursor")?;
        let choices = match value.get("choice") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::String(choice)) => vec![choice.clone()],
            _ => return Err("Invalid question draft choice".into()),
        };
        Ok(Self {
            other: choices.is_empty() && !text.trim().is_empty(),
            choices,
            text,
            caret,
            row: 0,
        })
    }
}

fn text(value: &Value, caret: &str) -> Result<(String, usize), String> {
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .ok_or("Invalid question draft text")?;
    let caret = value
        .get(caret)
        .and_then(Value::as_usize)
        .ok_or("Invalid question draft cursor")?;
    if text.len() > TEXT_LIMIT || caret > text.len() || !text.is_char_boundary(caret) {
        return Err("Invalid question draft".into());
    }
    Ok((text.into(), caret))
}
