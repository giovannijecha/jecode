//! Native workspace listing and literal text search.
use crate::{cancel::Cancellation, json::Value};
use std::path::Path;

mod page;
mod schema;
mod search;
mod text;
mod walk;

use page::Page;
pub(super) use schema::definitions;
use walk::Walk;

pub(super) fn execute(
    root: &Path,
    name: &str,
    arguments: &Value,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    if name == "search" {
        return search::execute(root, arguments, cancellation);
    }
    let recursive = boolean(arguments, "recursive", false)?;
    let mut walk = Walk::new(root, arguments, recursive, false, cancellation)?;
    let mut page = Page::new(arguments, 200, 2000)?;
    while let Some(entry) = walk.next()? {
        if !walk.selected(&entry)? {
            continue;
        }
        let value = Value::object([
            ("path", Value::string(&entry.relative)),
            ("type", Value::string(entry.kind)),
            ("depth", Value::number(entry.depth)),
            (
                "size",
                if entry.kind == "file" {
                    Value::number(entry.size)
                } else {
                    Value::Null
                },
            ),
        ]);
        if !page.offer(value)? {
            break;
        }
    }
    page.finish("entries", &walk, [])
}

fn boolean(arguments: &Value, name: &str, default: bool) -> Result<bool, String> {
    match arguments.get(name) {
        None => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        _ => Err(format!("{name} must be a boolean")),
    }
}

fn string<'a>(arguments: &'a Value, name: &str, default: &'a str) -> Result<&'a str, String> {
    match arguments.get(name) {
        None => Ok(default),
        Some(Value::String(value)) => Ok(value),
        _ => Err(format!("{name} must be a string")),
    }
}

#[cfg(test)]
mod regression_tests;
#[cfg(test)]
mod tests;
