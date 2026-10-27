//! Live structural orientation and durable, source-backed model annotations.
use crate::{cancel::Cancellation, json::Value, redact::Redactor, tools::discovery::page::Page};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

mod schema;
mod selection;
mod sources;
mod storage;
mod survey;

pub(crate) fn definitions() -> Vec<Value> {
    schema::definitions()
}

pub(crate) struct Map {
    root: PathBuf,
    store: storage::Store,
    redactor: Redactor,
}

impl Map {
    pub fn new(root: PathBuf) -> Self {
        Self {
            store: storage::Store::new(root.clone()),
            root,
            redactor: Redactor::empty(),
        }
    }

    pub fn configure(&mut self, home: &Path) {
        self.store.configure(home);
    }
    pub fn include_credentials(&mut self, redactor: Redactor) {
        self.redactor.include(redactor);
    }

    pub fn execute(&self, arguments: &Value, cancellation: &Cancellation) -> Result<Value, String> {
        if cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        let result = match arguments.get("action") {
            None => self.overview(arguments, cancellation),
            Some(Value::String(action)) => match action.as_str() {
                "overview" => self.overview(arguments, cancellation),
                "remember" => self.remember(arguments, cancellation),
                "forget" => self.forget(arguments, cancellation),
                _ => Err(format!("Unknown map action: {action}")),
            },
            _ => Err("map action must be a string".into()),
        };
        result
            .map(|value| self.redactor.value(&value))
            .map_err(|error| self.redactor.text(&error))
    }

    fn overview(&self, args: &Value, cancellation: &Cancellation) -> Result<Value, String> {
        let mut page = Page::new(args, 100, 1000)?;
        let mut survey = survey::collect(&self.root, args, cancellation)?;
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();
        let (notes, notes_available) = match self.store.read() {
            Ok(notes) => (notes, true),
            Err(error) => {
                survey.errors_count += 1;
                survey.errors.push(Value::object([
                    ("path", Value::string("project map notes")),
                    ("error", Value::string(self.redactor.text(&error))),
                ]));
                (Vec::new(), false)
            }
        };
        let notes_count = notes.len();
        let mut sources = sources::Sources::new(&self.root, cancellation);
        let mut selection =
            selection::Selection::new(&self.root, &survey.scope, args, cancellation);
        let mut entries = Vec::new();
        for note in notes {
            let selected = selection.note(&note)?;
            if selected
                && (query.is_empty()
                    || ["title", "summary"]
                        .into_iter()
                        .filter_map(|key| note.get(key).and_then(Value::as_str))
                        .any(|text| text.to_lowercase().contains(&query))
                    || note
                        .get("sources")
                        .and_then(Value::as_array)
                        .is_some_and(|sources| {
                            sources
                                .iter()
                                .filter_map(|source| source.get("path").and_then(Value::as_str))
                                .any(|path| path.to_lowercase().contains(&query))
                        }))
            {
                entries.push(note);
            }
        }
        entries.extend(survey.entries);
        for entry in entries {
            if !page.offer_with(|| {
                let value = if entry.get("kind").and_then(Value::as_str) == Some("note") {
                    sources.refresh(&entry)?
                } else {
                    entry
                };
                Ok(self.redactor.value(&value))
            })? {
                break;
            }
        }
        page.finish_with(
            "entries",
            BTreeMap::from([
                ("action".into(), Value::string("overview")),
                ("path".into(), Value::string(survey.scope)),
                ("files".into(), Value::number(survey.files)),
                (
                    "notes_count".into(),
                    if notes_available {
                        Value::number(notes_count)
                    } else {
                        Value::Null
                    },
                ),
                ("notes_available".into(), Value::Bool(notes_available)),
                (
                    "complete".into(),
                    Value::Bool(!survey.truncated && survey.errors_count == 0),
                ),
                ("truncated".into(), Value::Bool(survey.truncated)),
                ("inventory_truncated".into(), Value::Bool(survey.truncated)),
                ("skipped".into(), survey.skipped),
                ("errors_count".into(), Value::number(survey.errors_count)),
                (
                    "errors_truncated".into(),
                    Value::Bool(survey.errors_count > survey.errors.len()),
                ),
                ("errors".into(), Value::Array(survey.errors)),
            ]),
        )
    }

    fn remember(&self, args: &Value, cancellation: &Cancellation) -> Result<Value, String> {
        let title = bounded_text(args, "title", 200)?;
        let summary = bounded_text(args, "summary", 8192)?;
        let id = match args.get("id") {
            None => unique_id(),
            Some(_) => note_id(args)?.to_owned(),
        };
        let references = args
            .get("sources")
            .and_then(Value::as_array)
            .filter(|sources| !sources.is_empty() && sources.len() <= 16)
            .ok_or("remember needs 1-16 source references")?;
        let mut reader = sources::Sources::new(&self.root, cancellation);
        let sources = references
            .iter()
            .map(|source| reader.capture(source, &self.redactor))
            .collect::<Result<Vec<_>, _>>()?;
        let note = self.redactor.value(&Value::object([
            ("kind", Value::string("note")),
            ("id", Value::string(&id)),
            ("title", Value::string(title)),
            ("summary", Value::string(summary)),
            ("origin", Value::string("model")),
            ("sources", Value::Array(sources)),
            ("freshness", Value::string("current")),
        ]));
        if note.encode().len() > 48 * 1024 {
            return Err(
                "Note exceeds the result budget; shorten its summary or source list".into(),
            );
        }
        self.store.update(cancellation, |notes| {
            if let Some(existing) = notes
                .iter_mut()
                .find(|note| note.get("id").and_then(Value::as_str) == Some(&id))
            {
                *existing = note.clone();
            } else {
                notes.push(note.clone());
            }
            Ok(())
        })?;
        Ok(Value::object([
            ("action", Value::string("remember")),
            ("id", Value::string(id)),
            ("note", note),
            ("saved", Value::Bool(true)),
        ]))
    }

    fn forget(&self, args: &Value, cancellation: &Cancellation) -> Result<Value, String> {
        let id = note_id(args)?;
        let removed = self.store.update(cancellation, |notes| {
            let count = notes.len();
            notes.retain(|note| note.get("id").and_then(Value::as_str) != Some(id));
            Ok(notes.len() < count)
        })?;
        Ok(Value::object([
            ("action", Value::string("forget")),
            ("id", Value::string(id)),
            ("removed", Value::Bool(removed)),
        ]))
    }
}

fn bounded_text<'a>(args: &'a Value, name: &str, limit: usize) -> Result<&'a str, String> {
    let text = crate::tools::required_string(args, name)?;
    if text.trim().is_empty() || text.len() > limit {
        return Err(format!("{name} must contain 1-{limit} UTF-8 bytes"));
    }
    Ok(text)
}

fn note_id(args: &Value) -> Result<&str, String> {
    let id = bounded_text(args, "id", 64)?;
    if !id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        return Err("note id may contain only ASCII letters, digits, '.', '_' and '-'".into());
    }
    Ok(id)
}

fn unique_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!(
        "{now}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests;
