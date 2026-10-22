//! The user's checkpoint commands: `jecode checkpoints ...` without a model
//! provider, and `/checkpoints ...` inside a session. Both parse the same
//! words into actions and render the results as labeled rows.
use super::actions::{self, Caller};
use super::render::{self, Output};
use super::show::{self, Content};
use super::workspace::{self, Options, Workspace};
use crate::json::Value;
use std::path::Path;
use std::sync::Arc;

pub const USAGE: &str = "\
status                              archive state, limits and activity
list [PATH...] [--limit N]          points, or the recorded versions of paths
capture PATH...                     save explicit file paths before Bash
inspect cp-N [PATH...]              a point and what restoring it would change
show cp-N PATH                      a path's recorded content
milestone LABEL [--pin]             record a named point
restore cp-N PATH... [--exact-paths] [--preview] [--expected cp-M|latest]
restore-all cp-N [--preview] [--expected cp-M|latest]
pin cp-N [LABEL] | unpin cp-N       keep a point regardless of the budget
config [--budget SIZE] [--max-file SIZE|unlimited]
gc                                  release history beyond the budget now
verify                              reread every file for disguised edits";

/// Lines shown for a recorded text file; the plain command prints it whole.
const SHOWN_LINES: usize = 2000;

/// Parsed words: action arguments, or a `show` request.
pub enum Parsed {
    Action(Value),
    Show(String),
}

/// Splits command text into words; double quotes keep spaces, `\"` a quote.
pub fn split(text: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut started = false;
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' if quoted => match chars.next() {
                Some(next @ ('"' | '\\')) => word.push(next),
                Some(next) => {
                    word.push('\\');
                    word.push(next);
                }
                None => word.push('\\'),
            },
            '"' => {
                quoted = !quoted;
                started = true;
            }
            ch if ch.is_whitespace() && !quoted => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            ch => {
                word.push(ch);
                started = true;
            }
        }
    }
    if quoted {
        return Err("A quoted word is not closed".into());
    }
    if started {
        words.push(word);
    }
    Ok(words)
}

pub fn parse(words: &[String]) -> Result<Parsed, String> {
    let mut words = words.iter().map(String::as_str);
    let action = words.next().unwrap_or("status");
    let mut positional = Vec::new();
    let mut fields: Vec<(&str, Value)> = Vec::new();
    while let Some(word) = words.next() {
        let mut value = |name: &str| {
            words
                .next()
                .map(Value::string)
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match word {
            "--preview" => fields.push(("preview", Value::Bool(true))),
            "--exact-paths" => fields.push(("exact", Value::Bool(true))),
            "--pin" => fields.push(("pin", Value::Bool(true))),
            "--expected" => fields.push(("expected", value(word)?)),
            "--budget" => fields.push(("budget", value(word)?)),
            "--max-file" => fields.push(("max_file", value(word)?)),
            "--limit" => {
                let limit = value(word)?;
                let number = limit.as_str().and_then(|text| text.parse::<usize>().ok());
                fields.push((
                    "limit",
                    Value::number(number.ok_or("--limit needs a number")?),
                ));
            }
            flag if flag.starts_with("--") => return Err(format!("Unknown option {flag}")),
            other => positional.push(other),
        }
    }
    let mut positional = positional.into_iter();
    let action = match action {
        "restore-all" => "restore_all",
        "show" => {
            let (Some(target), Some(path), None) =
                (positional.next(), positional.next(), positional.next())
            else {
                return Err("show needs cp-N and one path".into());
            };
            return Ok(Parsed::Show(format!("{}{target}:{path}", show::PREFIX)));
        }
        other => other,
    };
    if matches!(
        action,
        "inspect" | "restore" | "restore_all" | "pin" | "unpin"
    ) {
        let target = positional
            .next()
            .ok_or_else(|| format!("{action} needs a checkpoint such as cp-12"))?;
        fields.push(("ref", Value::string(target)));
    }
    let rest: Vec<&str> = positional.collect();
    match action {
        "list" | "inspect" | "restore" | "capture" if !rest.is_empty() => {
            fields.push((
                "paths",
                Value::Array(rest.into_iter().map(Value::string).collect()),
            ));
        }
        "milestone" | "pin" if !rest.is_empty() => {
            fields.push(("label", Value::string(rest.join(" "))));
        }
        "milestone" => return Err("milestone needs a label".into()),
        "capture" => return Err("capture needs explicit file paths".into()),
        "status" | "list" | "inspect" | "restore" | "restore_all" | "pin" | "unpin" | "config"
        | "gc" | "verify" => {
            if let Some(word) = rest.first() {
                return Err(format!("Unexpected {word:?} after {action}"));
            }
        }
        other => return Err(format!("Unknown checkpoints command {other:?}\n{USAGE}")),
    }
    fields.push(("action", Value::string(action)));
    Ok(Parsed::Action(super::session::object(fields)))
}

/// Runs the words against a workspace and renders the result for `prefix`
/// (`jecode checkpoints` or `/checkpoints`).
pub fn execute(
    workspace: &Arc<Workspace>,
    origin: &str,
    words: &[String],
    prefix: &str,
) -> Result<Output, String> {
    if words.first().is_some_and(|word| word == "help") {
        return Ok(help(prefix));
    }
    match parse(words)? {
        Parsed::Show(reference) => {
            let title = reference
                .strip_prefix(show::PREFIX)
                .unwrap_or(&reference)
                .to_string();
            Ok(match show::content(workspace, &reference)? {
                Content::Text(text) => shown(title, text),
                Content::Preview(text, length) => {
                    let mut result = shown(title, text);
                    result.raw = None;
                    result.details.push(("preview".into(), format!("{length} bytes total; use checkpoint read pages or jecode checkpoints show for exact full content")));
                    result
                }
                Content::Binary(length) => output(
                    title,
                    vec![(
                        "binary".into(),
                        format!("{length} bytes; restore it to inspect"),
                    )],
                ),
                Content::Directory(children) => output(
                    title,
                    children
                        .into_iter()
                        .map(|child| (child, String::new()))
                        .collect(),
                ),
                Content::Link(target) => output(title, vec![("link".into(), target)]),
            })
        }
        Parsed::Action(arguments) => {
            let caller = Caller { origin, user: true };
            let result = actions::run(workspace, &caller, &arguments)?;
            let summary = crate::events::tool_summary("checkpoint", &result);
            Ok(render::output(
                format!("Checkpoints · {summary}"),
                &result,
                prefix,
            ))
        }
    }
}

fn output(title: String, details: Vec<(String, String)>) -> Output {
    Output {
        title,
        details,
        raw: None,
    }
}

/// A recorded text file: numbered rows for the panel, exact text for print.
fn shown(title: String, text: String) -> Output {
    let lines: Vec<&str> = text.lines().collect();
    let mut details: Vec<(String, String)> = lines
        .iter()
        .take(SHOWN_LINES)
        .enumerate()
        .map(|(index, line)| ((index + 1).to_string(), (*line).to_string()))
        .collect();
    if lines.len() > SHOWN_LINES {
        details.push((
            "…".into(),
            format!(
                "{} more lines; jecode checkpoints show prints the whole file",
                lines.len() - SHOWN_LINES
            ),
        ));
    }
    Output {
        title,
        details,
        raw: Some(text),
    }
}

fn help(prefix: &str) -> Output {
    output(
        format!("{prefix} commands"),
        USAGE
            .lines()
            .map(|line| match line.split_once("  ") {
                Some((usage, description)) => {
                    (usage.trim().to_string(), description.trim().to_string())
                }
                None => (line.to_string(), String::new()),
            })
            .collect(),
    )
}

/// `jecode checkpoints ...`: works without a model provider or network.
pub fn offline(home: &Path, root: &Path, words: &[String]) -> Result<Output, String> {
    let action = words.first().map_or("status", String::as_str);
    if action == "help" {
        return Ok(help("jecode checkpoints"));
    }
    let workspace = workspace::open(home, root, Options::offline())?;
    // Commands that compare with the workspace first reconcile it.
    if !matches!(
        action,
        "status" | "list" | "show" | "pin" | "unpin" | "config" | "gc" | "capture"
    ) {
        workspace.initialize()?;
    }
    execute(&workspace, "cli", words, "jecode checkpoints")
}

/// Plain `show` streams complete text rather than building a giant panel.
pub fn stream_show(
    home: &Path,
    root: &Path,
    words: &[String],
    output: &mut impl std::io::Write,
) -> Result<bool, String> {
    if words.first().is_none_or(|word| word != "show") {
        return Ok(false);
    }
    let Parsed::Show(reference) = parse(words)? else {
        return Ok(false);
    };
    let workspace = workspace::open(home, root, Options::offline())?;
    show::write_text(&workspace, &reference, output)
}
