//! `jecode skills`: the personal library `/skills` lists, scanned read-only.
//! A missing library is empty; nothing is created.

use super::{Context, Report};
use crate::cli::{Failure, Manage};
use crate::json::Value;
use crate::skills::Library;

#[cfg(test)]
mod tests;

pub fn run(context: &Context, command: &Manage) -> Result<Report, Failure> {
    let directory = context.home.join("skills");
    let mut library = Library::at(Some(directory.clone()));
    library.refresh();
    let report: Result<Report, Failure> = match command.action {
        "list" => {
            let skills = library.list();
            let (summary, details) = crate::session::commands::skills_catalog(
                skills
                    .iter()
                    .map(|skill| (skill.name.clone(), skill.description.clone())),
                library.warnings(),
            );
            let mut text = format!("{summary}\n");
            for (name, description) in &details {
                text.push_str(&format!("{name}  {description}\n"));
            }
            text.push_str(&format!("Library: {}", directory.display()));
            Ok(Report::new(
                Value::object([
                    ("directory", Value::string(directory.to_string_lossy())),
                    (
                        "skills",
                        Value::Array(
                            skills
                                .iter()
                                .map(|skill| {
                                    Value::object([
                                        ("name", Value::string(&skill.name)),
                                        ("description", Value::string(&skill.description)),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                    (
                        "warnings",
                        Value::Array(library.warnings().iter().map(Value::string).collect()),
                    ),
                ]),
                text,
            ))
        }
        "show" => {
            let name = &command.operands[0];
            let (package, text) = library.source(name)?;
            Ok(Report::new(
                Value::object([
                    ("name", Value::string(name)),
                    ("path", Value::string(package.to_string_lossy())),
                    ("text", Value::string(&text)),
                ]),
                text,
            ))
        }
        action => unreachable!("skills {action}"),
    };
    Ok(report?.redact(&context.redactor()?))
}
