//! Provider-free CLI and conversation commands for per-call recovery.
use super::actions;
use super::render::{self, Output};
use super::{Store, show};
use crate::cancel::Cancellation;
use crate::json::Value;
use std::path::Path;
use std::sync::Arc;

pub const USAGE: &str = "\
list [--limit N]                    saved calls from the last 24 hours
inspect rc-ID [PATH...]              before/current states and restore token
show rc-ID PATH [--offset N | --byte-offset N] [--limit N]
                                     stream text, or show one page with options
restore rc-ID [PATH...] [--preview] [--expected TOKEN|latest]
Use -- before paths that start with --. CLI commands use PowerShell quoting on Windows.";

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

enum Parsed {
    Action(Value),
    Show {
        reference: String,
        page: Value,
        paged: bool,
        path: String,
    },
}

fn parse(words: &[String]) -> Result<Parsed, String> {
    let action = words.first().map_or("list", String::as_str);
    let mut fields = vec![("action", Value::string(action))];
    let mut page = Vec::new();
    let mut paged = false;
    let mut positional = Vec::new();
    let mut words = words.iter().skip(1);
    let mut literal = false;
    while let Some(word) = words.next() {
        if literal {
            positional.push(word);
            continue;
        }
        match word.as_str() {
            "--" => literal = true,
            "--preview" => fields.push(("preview", Value::Bool(true))),
            "--expected" => fields.push((
                "expected",
                Value::string(words.next().ok_or("--expected needs a token or latest")?),
            )),
            "--limit" => {
                let limit = words
                    .next()
                    .and_then(|value| value.parse::<usize>().ok())
                    .ok_or("--limit needs a number")?;
                fields.push(("limit", Value::number(limit)));
                if action == "show" {
                    page.push(("limit", Value::number(limit)));
                    paged = true;
                }
            }
            "--offset" | "--byte-offset" if action == "show" => {
                let number = words
                    .next()
                    .and_then(|value| value.parse::<usize>().ok())
                    .ok_or_else(|| format!("{word} needs a number"))?;
                page.push((
                    if word == "--offset" {
                        "offset"
                    } else {
                        "byte_offset"
                    },
                    Value::number(number),
                ));
                paged = true;
            }
            flag if flag.starts_with("--") => return Err(format!("Unknown option {flag}")),
            _ => positional.push(word),
        }
    }
    match action {
        "show" if positional.len() == 2 => Ok(Parsed::Show {
            reference: format!("{}{}:{}", show::PREFIX, positional[0], positional[1]),
            page: object(page),
            paged,
            path: positional[1].to_string(),
        }),
        "inspect" | "restore" if !positional.is_empty() => {
            fields.push(("ref", Value::string(positional[0])));
            if positional.len() > 1 {
                fields.push((
                    "paths",
                    Value::Array(
                        positional[1..]
                            .iter()
                            .map(|path| Value::string(path.as_str()))
                            .collect(),
                    ),
                ));
            }
            Ok(Parsed::Action(object(fields)))
        }
        "list" if positional.is_empty() => Ok(Parsed::Action(object(fields))),
        "show" | "inspect" | "restore" => {
            Err(format!("{action} needs a recovery reference; {USAGE}"))
        }
        _ => Err(format!("Unknown checkpoints command {action:?}\n{USAGE}")),
    }
}

fn object(fields: Vec<(&str, Value)>) -> Value {
    Value::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect(),
    )
}

pub fn run(store: &Arc<Store>, words: &[String]) -> Result<Output, String> {
    execute(store, words, "/checkpoints")
}

fn execute(store: &Arc<Store>, words: &[String], prefix: &str) -> Result<Output, String> {
    if words.first().is_some_and(|word| word == "help") {
        return Ok(help(prefix));
    }
    let cancel = Cancellation::default();
    match parse(words)? {
        Parsed::Show {
            reference,
            page,
            path,
            ..
        } => {
            let value = show::read(store, &reference, &page, &cancel)?;
            let mut output = render::output(reference.clone(), &value, prefix);
            output.raw = value
                .get("content")
                .and_then(Value::as_str)
                .map(String::from);
            if value.get("truncated") == Some(&Value::Bool(true)) {
                let mut words = vec![prefix.to_string(), "show".into()];
                let (call, _) = reference
                    .trim_start_matches(show::PREFIX)
                    .split_once(':')
                    .expect("parsed show reference");
                words.push(call.into());
                if value.get("partial_line") == Some(&Value::Bool(true))
                    || page.get("byte_offset").is_some()
                {
                    let next = value
                        .get("next_byte_offset")
                        .and_then(Value::as_usize)
                        .expect("truncated page has next byte");
                    words.extend(["--byte-offset".into(), next.to_string()]);
                } else {
                    let next = value
                        .get("next_offset")
                        .and_then(Value::as_usize)
                        .expect("truncated page has next line");
                    words.extend(["--offset".into(), next.to_string()]);
                }
                if let Some(limit) = page.get("limit").and_then(Value::as_usize) {
                    words.extend(["--limit".into(), limit.to_string()]);
                }
                words.extend(["--".into(), render::quote(&path, prefix)]);
                let next = words.join(" ");
                output.details.push(("next page".into(), next.clone()));
                if let Some(raw) = output.raw.as_mut() {
                    if !raw.ends_with('\n') {
                        raw.push('\n');
                    }
                    raw.push_str(&format!("\nNext page: {next}\n"));
                }
            }
            Ok(output)
        }
        Parsed::Action(args) => {
            let value = actions::run(store, &args, &cancel)?;
            Ok(render::output("Checkpoints".into(), &value, prefix))
        }
    }
}

fn help(prefix: &str) -> Output {
    Output {
        title: format!("{prefix} commands"),
        details: USAGE
            .lines()
            .map(|line| (line.into(), String::new()))
            .collect(),
        raw: None,
    }
}

pub fn offline(home: &Path, root: &Path, words: &[String]) -> Result<Output, String> {
    if words.first().is_some_and(|word| word == "help") {
        return Ok(help("jecode checkpoints"));
    }
    // Maintenance in another archive must not prevent local recovery.
    let _ = Store::sweep_home(home, super::clock::now(), &Cancellation::default());
    execute(&Store::open(home, root)?, words, "jecode checkpoints")
}

pub fn stream_show(
    home: &Path,
    root: &Path,
    words: &[String],
    output: &mut impl std::io::Write,
) -> Result<bool, String> {
    if words.first().is_none_or(|word| word != "show") {
        return Ok(false);
    }
    let Parsed::Show {
        reference, paged, ..
    } = parse(words)?
    else {
        return Ok(false);
    };
    if paged {
        return Ok(false);
    }
    let _ = Store::sweep_home(home, super::clock::now(), &Cancellation::default());
    show::write_text(&Store::open(home, root)?, &reference, output)?;
    Ok(true)
}

#[cfg(test)]
#[path = "command_tests.rs"]
mod tests;
