use super::{Pending, Record, State, apply};
use crate::json::{self, Value};
use crate::scratch::Area;
use std::collections::{HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
pub(super) fn manifest_file(area: &Area, id: &str) -> Result<std::path::PathBuf, String> {
    let (_, path) = area.file(&format!("web/crawls/{id}/manifest.jsonl"), true)?;
    if fs::symlink_metadata(&path).is_ok_and(|metadata| crate::disk::linked(&metadata)) {
        return Err("Crawl manifest contains a link".into());
    }
    Ok(path)
}

pub(super) fn append(area: &Area, id: &str, record: &Record) -> Result<(), String> {
    let path = manifest_file(area, id)?;
    let mut file = OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|error| format!("Could not open crawl manifest: {error}"))?;
    file.write_all(encode_record(record).encode().as_bytes())
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("Could not append crawl manifest: {error}"))
}

pub(super) fn replay(area: &Area, state: &mut State) -> Result<bool, String> {
    let path = manifest_file(area, &state.id)?;
    let file =
        fs::File::open(&path).map_err(|error| format!("Could not open crawl manifest: {error}"))?;
    let mut reader = BufReader::new(file);
    let mut count = 0;
    let mut complete_bytes = 0_u64;
    let mut repaired = false;
    loop {
        let mut chunk = Vec::new();
        let length = reader
            .read_until(b'\n', &mut chunk)
            .map_err(|error| format!("Could not read crawl manifest: {error}"))?;
        if length == 0 {
            break;
        }
        let complete = chunk.ends_with(b"\n");
        let record = std::str::from_utf8(&chunk)
            .ok()
            .and_then(|source| json::parse(source).ok())
            .and_then(|value| decode_record(value).ok());
        let Some(record) = record else {
            if complete {
                return Err(format!("Invalid crawl manifest record {}", count + 1));
            }
            if count < state.pages_total {
                return Err("Crawl manifest is behind saved state".into());
            }
            drop(reader);
            OpenOptions::new()
                .write(true)
                .open(&path)
                .and_then(|file| file.set_len(complete_bytes))
                .map_err(|error| {
                    format!("Could not remove incomplete crawl manifest tail: {error}")
                })?;
            repaired = true;
            break;
        };
        if count >= state.pages_total {
            apply(state, &record)?;
        }
        count += 1;
        complete_bytes += length as u64;
        if !complete {
            drop(reader);
            OpenOptions::new()
                .append(true)
                .open(&path)
                .and_then(|mut file| file.write_all(b"\n").and_then(|()| file.sync_all()))
                .map_err(|error| format!("Could not finish crawl manifest record: {error}"))?;
            repaired = true;
            break;
        }
    }
    if count < state.pages_total {
        return Err("Crawl manifest is behind saved state".into());
    }
    Ok(repaired)
}

fn encode_record(record: &Record) -> Value {
    Value::object([
        ("page", record.page.clone()),
        (
            "discovered",
            Value::Array(record.discovered.iter().map(encode_pending).collect()),
        ),
    ])
}

fn decode_record(value: Value) -> Result<Record, String> {
    let page = value
        .get("page")
        .ok_or("Invalid crawl manifest page")?
        .clone();
    let discovered = value
        .get("discovered")
        .and_then(Value::as_array)
        .ok_or("Invalid crawl manifest links")?
        .iter()
        .map(decode_pending)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Record { page, discovered })
}

fn encode_pending(item: &Pending) -> Value {
    Value::object([
        ("url", Value::string(&item.url)),
        ("depth", Value::number(item.depth)),
    ])
}

fn decode_pending(item: &Value) -> Result<Pending, String> {
    Ok(Pending {
        url: item
            .get("url")
            .and_then(Value::as_str)
            .ok_or("Invalid queued URL")?
            .to_string(),
        depth: item
            .get("depth")
            .and_then(Value::as_usize)
            .ok_or("Invalid queued depth")?,
    })
}

pub(super) fn integer(
    arguments: &Value,
    key: &str,
    default: usize,
    positive: bool,
) -> Result<usize, String> {
    match arguments.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_usize()
            .filter(|value| !positive || *value > 0)
            .ok_or_else(|| {
                format!(
                    "{key} must be {} integer",
                    if positive {
                        "a positive"
                    } else {
                        "a nonnegative"
                    }
                )
            }),
    }
}

pub(super) fn boolean(arguments: &Value, key: &str, default: bool) -> Result<bool, String> {
    match arguments.get(key) {
        None => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        _ => Err(format!("{key} must be a boolean")),
    }
}

pub(super) fn save(area: &Area, state: &mut State) -> Result<(), String> {
    let next = state.sequence + 1;
    let alias = format!("web/crawls/{}/state-{}.json", state.id, next % 2);
    let (_, path) = area.file(&alias, true)?;
    if fs::symlink_metadata(&path).is_ok_and(|metadata| crate::disk::linked(&metadata)) {
        return Err("Crawl state contains a link".into());
    }
    fs::write(path, encode(state, next).encode())
        .map_err(|error| format!("Could not save crawl state: {error}"))?;
    state.sequence = next;
    Ok(())
}

fn encode(state: &State, sequence: usize) -> Value {
    let mut visited: Vec<_> = state.visited.iter().cloned().collect();
    visited.sort();
    Value::object([
        ("format", Value::string("jecode.web.crawl")),
        ("sequence", Value::number(sequence)),
        ("crawl_id", Value::string(&state.id)),
        (
            "seeds",
            Value::Array(state.seeds.iter().map(Value::string).collect()),
        ),
        ("same_origin", Value::Bool(state.same_origin)),
        ("max_depth", Value::number(state.max_depth)),
        (
            "queue",
            Value::Array(state.queue.iter().map(encode_pending).collect()),
        ),
        (
            "visited",
            Value::Array(visited.iter().map(Value::string).collect()),
        ),
        ("pages_total", Value::number(state.pages_total)),
    ])
}

pub(super) fn load(area: &Area, id: &str) -> Result<State, String> {
    if id.is_empty()
        || id.len() > 80
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("Invalid crawl_id".into());
    }
    let mut candidates = Vec::new();
    for slot in 0..2 {
        let (_, path) = area.file(&format!("web/crawls/{id}/state-{slot}.json"), true)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) if crate::disk::linked(&metadata) => {
                return Err("Crawl state contains a link".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("Could not inspect crawl state: {error}")),
        }
        if let Ok(source) = fs::read_to_string(path)
            && let Ok(value) = json::parse(&source)
            && let Ok(state) = decode(value, id)
        {
            candidates.push(state);
        }
    }
    candidates
        .into_iter()
        .max_by_key(|state| state.sequence)
        .ok_or("Crawl state was not found or is invalid".into())
}

fn decode(value: Value, id: &str) -> Result<State, String> {
    if value.get("format").and_then(Value::as_str) != Some("jecode.web.crawl")
        || value.get("crawl_id").and_then(Value::as_str) != Some(id)
    {
        return Err("Invalid crawl state".into());
    }
    let sequence = value
        .get("sequence")
        .and_then(Value::as_usize)
        .ok_or("Invalid crawl sequence")?;
    let seeds = value
        .get("seeds")
        .and_then(Value::as_array)
        .ok_or("Invalid crawl seeds")?
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .ok_or("Invalid crawl seed".into())
        })
        .collect::<Result<Vec<_>, String>>()?;
    let same_origin = match value.get("same_origin") {
        Some(Value::Bool(value)) => *value,
        _ => return Err("Invalid crawl scope".into()),
    };
    let max_depth = value
        .get("max_depth")
        .and_then(Value::as_usize)
        .ok_or("Invalid crawl depth")?;
    let queue = value
        .get("queue")
        .and_then(Value::as_array)
        .ok_or("Invalid crawl queue")?
        .iter()
        .map(decode_pending)
        .collect::<Result<VecDeque<_>, _>>()?;
    let visited = value
        .get("visited")
        .and_then(Value::as_array)
        .ok_or("Invalid crawl visited set")?
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .ok_or("Invalid visited URL".into())
        })
        .collect::<Result<HashSet<_>, String>>()?;
    let pages_total = value
        .get("pages_total")
        .and_then(Value::as_usize)
        .ok_or("Invalid crawl page count")?;
    Ok(State {
        id: id.into(),
        sequence,
        seeds,
        same_origin,
        max_depth,
        queue,
        visited,
        pages_total,
    })
}
