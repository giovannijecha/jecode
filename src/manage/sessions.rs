//! `jecode sessions`: this folder's saved conversations, read without
//! opening them. Deletion takes the same lease as a writer, so an open
//! conversation is refused; unsupported files are reported and left alone.

use super::{Context, Report, confirm};
use crate::cli::{Failure, Manage, usage};
use crate::json::Value;
use crate::redact::Redactor;
use crate::sessions::{Document, Record, Store, Summary};
use std::io::Write;
use std::sync::{Arc, Mutex};

pub fn run(
    context: &Context,
    command: &Manage,
    output: &mut impl Write,
) -> Result<Report, Failure> {
    let directory = std::env::current_dir().map_err(|error| error.to_string())?;
    run_in(context, command, &directory, output)
}

pub fn run_in(
    context: &Context,
    command: &Manage,
    directory: &std::path::Path,
    output: &mut impl Write,
) -> Result<Report, Failure> {
    let store = Store::new(context.home.clone(), directory)?;
    match command.action {
        "list" => list(context, &store),
        "show" => show(context, &store, id(&command.operands[0])?),
        "export" => export(
            context,
            &store,
            id(&command.operands[0])?,
            command.value("output"),
            output,
        ),
        "delete" => delete(
            context,
            &store,
            id(&command.operands[0])?,
            command.flag("yes"),
        ),
        action => unreachable!("sessions {action}"),
    }
}

fn id(value: &str) -> Result<&str, Failure> {
    if crate::sessions::valid_id(value) {
        Ok(value)
    } else {
        Err(usage(format!(
            "'{value}' is not a session ID; list them with jecode sessions list"
        )))
    }
}

fn summary_value(summary: &Summary) -> Value {
    Value::object([
        ("id", Value::string(&summary.id)),
        ("title", Value::string(&summary.title)),
        ("updated_at_unix_ms", Value::number(summary.updated)),
        ("provider", Value::string(&summary.backend)),
        ("model", Value::string(&summary.model)),
    ])
}

fn list(context: &Context, store: &Store) -> Result<Report, Failure> {
    let listing = store.list_read_only()?;
    let mut text = String::new();
    for session in &listing.sessions {
        text.push_str(&format!(
            "{}  {:>8}  {} / {}  {}\n",
            session.id,
            session.elapsed(),
            session.backend,
            session.model,
            session.title
        ));
    }
    if listing.sessions.is_empty() {
        text.push_str("No saved conversations in this folder.\n");
    }
    for warning in &listing.warnings {
        text.push_str(&format!("{warning}\n"));
    }
    text.push_str("Continue one with jecode resume SESSION_ID, or jecode resume --last.");
    let redactor = context.redactor()?;
    let report = Report::new(
        Value::object([
            (
                "directory",
                Value::string(store.directory().to_string_lossy()),
            ),
            (
                "sessions",
                Value::Array(listing.sessions.iter().map(summary_value).collect()),
            ),
            (
                "warnings",
                Value::Array(listing.warnings.iter().map(Value::string).collect()),
            ),
        ]),
        text,
    );
    Ok(report.redact(&redactor))
}

fn show(context: &Context, store: &Store, id: &str) -> Result<Report, Failure> {
    let document = store.read(id)?;
    let redactor = context.redactor()?;
    let mut messages = Vec::new();
    let mut transcript = String::new();
    for record in document.records() {
        match record {
            Record::Text { role, text } => {
                transcript.push_str(&if role == "user" {
                    format!("\n> {text}\n")
                } else {
                    format!("\n{text}\n")
                });
                messages.push(Value::object([
                    ("role", Value::string(role)),
                    ("text", Value::string(text)),
                ]));
            }
            Record::Tool {
                name,
                arguments,
                summary,
                ..
            } => {
                transcript.push_str(&format!(
                    "[tool: {name}] {} · {summary}\n",
                    crate::events::description(&name, &arguments)
                ));
            }
            Record::Local {
                command, result, ..
            } => {
                transcript.push_str(&format!("\n{command}\n  {result}\n"));
            }
            Record::Question { .. }
            | Record::Collaboration { .. }
            | Record::ProcessFinished { .. }
            | Record::TurnStart => {}
        }
    }
    let level = document
        .permissions
        .map_or("workspace".to_owned(), |level| level.id().to_owned());
    let text = format!(
        "{}\nID:          {}\nFolder:      {}\nProvider:    {}\nModel:       {} · effort {}\nPermissions: {level}\nUpdated:     {}\n{transcript}",
        document.title(),
        document.id,
        document.directory.display(),
        document.backend,
        document.model_label(),
        document.effort.name(),
        Summary {
            id: String::new(),
            title: String::new(),
            updated: document.updated,
            backend: String::new(),
            model: String::new(),
        }
        .age(),
    );
    let value = Value::object([
        ("id", Value::string(&document.id)),
        ("title", Value::string(document.title())),
        (
            "directory",
            Value::string(document.directory.to_string_lossy()),
        ),
        ("provider", Value::string(&document.backend)),
        ("model", Value::optional(document.model.clone())),
        ("effort", Value::string(document.effort.name())),
        ("permissions", Value::string(&level)),
        ("created_at_unix_ms", Value::number(document.created)),
        ("updated_at_unix_ms", Value::number(document.updated)),
        ("messages", Value::Array(messages)),
    ]);
    Ok(Report::new(value, text).redact(&redactor))
}

fn archive(document: &Document, redactor: Redactor) -> crate::export::Archive {
    crate::export::Archive {
        backend: document.backend.clone(),
        model: document.model_label().into(),
        directory: document.directory.clone(),
        messages: Arc::new(Mutex::new(document.messages.clone())),
        redactor,
        effort: document.effort.name().into(),
        events: Arc::new(Mutex::new(document.events.clone())),
    }
}

fn export(
    context: &Context,
    store: &Store,
    id: &str,
    target: Option<&str>,
    output: &mut impl Write,
) -> Result<Report, Failure> {
    let document = store.read(id)?;
    let archive = archive(&document, context.redactor()?);
    let path = match target {
        Some("-") => {
            let text = format!("{}\n", archive.document().pretty());
            output
                .write_all(text.as_bytes())
                .and_then(|()| output.flush())
                .map_err(|error| error.to_string())?;
            return Ok(Report::done());
        }
        Some(path) => {
            let path = std::path::PathBuf::from(path);
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|error| {
                    format!(
                        "Could not create {}: {error}; an existing file is never replaced",
                        path.display()
                    )
                })?;
            let written = file
                .write_all(format!("{}\n", archive.document().pretty()).as_bytes())
                .and_then(|()| file.sync_all());
            if let Err(error) = written {
                drop(file);
                let _ = std::fs::remove_file(&path);
                return Err(format!("Could not write {}: {error}", path.display()).into());
            }
            path
        }
        None => archive.save()?,
    };
    Ok(Report::new(
        Value::object([
            ("id", Value::string(id)),
            ("path", Value::string(path.to_string_lossy())),
        ]),
        format!("Exported conversation to {}", path.display()),
    ))
}

fn delete(context: &Context, store: &Store, id: &str, yes: bool) -> Result<Report, Failure> {
    // Reading first rejects missing, unsupported and foreign sessions unchanged.
    let document = store.read(id)?;
    // As in `show`: a key saved after the title was written is masked too.
    let title = context.redactor()?.text(&document.title());
    if !confirm(
        yes,
        &format!(
            "Delete {title} · {id}?\nThe conversation, its owned tool outputs and temporary files will be removed."
        ),
        "delete",
    )? {
        return Err("Cancelled; the conversation was kept".into());
    }
    let report = store.delete(id)?;
    let text = crate::session::delete::message(&title, &report, false);
    Ok(Report::new(
        Value::object([
            ("id", Value::string(id)),
            ("deleted", Value::Bool(true)),
            ("files", Value::number(report.files)),
            ("directories", Value::number(report.directories)),
            (
                "kept",
                Value::Array(
                    report
                        .kept
                        .iter()
                        .map(|path| Value::string(path.to_string_lossy()))
                        .collect(),
                ),
            ),
        ]),
        text,
    ))
}
