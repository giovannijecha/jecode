//! The questions of one `ask_user` call.

use crate::json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub label: String,
    pub description: String,
    pub recommended: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub id: String,
    pub title: String,
    pub question: String,
    pub context: String,
    pub multiple: bool,
    pub options: Vec<Choice>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: String,
    pub questions: Vec<Question>,
}

impl Request {
    /// The call's questions from its arguments, `{"questions": [...]}`.
    pub fn parse(id: &str, arguments: &Value) -> Result<Self, String> {
        let questions = match arguments.get("questions") {
            Some(Value::Array(items)) if !items.is_empty() => items
                .iter()
                .map(Question::parse)
                .collect::<Result<Vec<_>, _>>()?,
            _ => return Err("questions must contain at least one question".into()),
        };
        for (index, question) in questions.iter().enumerate() {
            if questions[..index]
                .iter()
                .any(|other| other.id == question.id)
            {
                return Err("Question IDs must be unique".into());
            }
        }
        Ok(Self {
            id: id.into(),
            questions,
        })
    }

    /// A saved `user_question` event with `questions`.
    pub(crate) fn from_event(event: &Value) -> Result<Self, String> {
        let id = event
            .get("tool_call_id")
            .and_then(Value::as_str)
            .ok_or("Missing question call ID")?;
        Self::parse(id, event)
    }

    /// The questions as call arguments and as the journal's `questions` field.
    pub fn arguments(&self) -> Value {
        Value::object([(
            "questions",
            Value::Array(self.questions.iter().map(Question::value).collect()),
        )])
    }

    /// The receipt's subject: the only question's title, or `N questions`.
    pub fn subject(&self) -> String {
        match self.questions.as_slice() {
            [only] => only.title.clone(),
            many => format!("{} questions", many.len()),
        }
    }
}

impl Question {
    fn parse(value: &Value) -> Result<Self, String> {
        let get = |key| value.get(key).and_then(Value::as_str);
        let id = get("id").ok_or("question id must be a string")?;
        let title = get("title").ok_or("title must be a string")?;
        let question = get("question").ok_or("question must be a string")?;
        let context = optional_text(value, "context")?;
        let multiple = match value.get("multiple") {
            None | Some(Value::Bool(false)) => false,
            Some(Value::Bool(true)) => true,
            _ => return Err("multiple must be a boolean".into()),
        };
        if id.is_empty() || title.trim().is_empty() || question.trim().is_empty() {
            return Err("Question ID, title and text must be nonempty".into());
        }
        let options = match value.get("options") {
            None => &[][..],
            Some(Value::Array(options)) => options.as_slice(),
            _ => return Err("options must be an array of choices".into()),
        };
        let mut parsed: Vec<Choice> = Vec::new();
        for option in options {
            let choice = Choice::parse(option)?;
            if parsed.iter().any(|item| item.id == choice.id) {
                return Err("Question option is invalid or duplicated".into());
            }
            parsed.push(choice);
        }
        Ok(Self {
            id: id.into(),
            title: title.into(),
            question: question.into(),
            context: context.into(),
            multiple,
            options: parsed,
        })
    }

    pub(crate) fn value(&self) -> Value {
        Value::object([
            ("id", Value::string(&self.id)),
            ("title", Value::string(&self.title)),
            ("question", Value::string(&self.question)),
            ("context", Value::string(&self.context)),
            ("multiple", Value::Bool(self.multiple)),
            (
                "options",
                Value::Array(self.options.iter().map(Choice::value).collect()),
            ),
        ])
    }

    /// Several choices count only when there are options to choose.
    pub fn is_multiple(&self) -> bool {
        self.multiple && !self.options.is_empty()
    }
}

impl Choice {
    fn parse(value: &Value) -> Result<Self, String> {
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .ok_or("option id must be a string")?;
        let label = value
            .get("label")
            .and_then(Value::as_str)
            .ok_or("option label must be a string")?;
        let description = optional_text(value, "description")?;
        let recommended = match value.get("recommended") {
            None | Some(Value::Bool(false)) => false,
            Some(Value::Bool(true)) => true,
            _ => return Err("recommended must be a boolean".into()),
        };
        if id.is_empty() || label.trim().is_empty() {
            return Err("Question option is invalid or duplicated".into());
        }
        Ok(Self {
            id: id.into(),
            label: label.into(),
            description: description.into(),
            recommended,
        })
    }

    fn value(&self) -> Value {
        Value::object([
            ("id", Value::string(&self.id)),
            ("label", Value::string(&self.label)),
            ("description", Value::string(&self.description)),
            ("recommended", Value::Bool(self.recommended)),
        ])
    }
}

fn optional_text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    match value.get(key) {
        None => Ok(""),
        Some(Value::String(text)) => Ok(text),
        Some(_) => Err(format!("{key} must be a string")),
    }
}
