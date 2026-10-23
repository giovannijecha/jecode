use super::describe_page;
use super::transport::{self, Options};
use super::url::Url;
use crate::cancel::Cancellation;
use crate::json::Value;
use crate::scratch::Area;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs::OpenOptions;

mod state;
use state::{append, boolean, integer, load, manifest_file, replay, save};

const RESULT_PAGES: usize = 20;

#[derive(Clone)]
struct Pending {
    url: String,
    depth: usize,
}

struct State {
    id: String,
    sequence: usize,
    seeds: Vec<String>,
    same_origin: bool,
    max_depth: usize,
    queue: VecDeque<Pending>,
    visited: HashSet<String>,
    pages_total: usize,
}

struct Record {
    page: Value,
    discovered: Vec<Pending>,
}

pub(super) fn execute(
    area: &Area,
    arguments: &Value,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    let (mut state, manifest_repaired) =
        if let Some(id) = arguments.get("crawl_id").and_then(Value::as_str) {
            let mut state = load(area, id)?;
            let repaired = replay(area, &mut state)?;
            (state, repaired)
        } else {
            let urls = arguments
                .get("urls")
                .and_then(Value::as_array)
                .ok_or("urls is required to start a crawl")?;
            if urls.is_empty() {
                return Err("urls must contain at least one URL".into());
            }
            let mut seeds = Vec::new();
            let mut seen = HashSet::new();
            for value in urls {
                let url = Url::parse(value.as_str().ok_or("urls must contain strings")?)?;
                if seen.insert(url.value.clone()) {
                    seeds.push(url.value);
                }
            }
            let id = transport::unique_id();
            let queue = seeds
                .iter()
                .map(|url| Pending {
                    url: url.clone(),
                    depth: 0,
                })
                .collect();
            let state = State {
                id,
                sequence: 0,
                seeds,
                same_origin: boolean(arguments, "same_origin", true)?,
                max_depth: integer(arguments, "max_depth", 1, false)?,
                queue,
                visited: HashSet::new(),
                pages_total: 0,
            };
            let path = manifest_file(area, &state.id)?;
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|error| format!("Could not create crawl manifest: {error}"))?;
            (state, false)
        };
    let max_pages = integer(arguments, "max_pages", 20, true)?;
    let options = Options::from(arguments)?;
    save(area, &mut state)?;
    let mut pages_in_call = 0;
    let mut fresh = Vec::new();
    let mut cancelled = false;
    while pages_in_call < max_pages {
        if cancellation.requested() {
            cancelled = true;
            break;
        }
        let Some(pending) = state.queue.front().cloned() else {
            break;
        };
        if state.visited.contains(&pending.url) {
            state.queue.pop_front();
            save(area, &mut state)?;
            continue;
        }
        let url = Url::parse(&pending.url)?;
        let result = transport::fetch(
            area,
            &url,
            &options,
            &format!("web/crawls/{}/pages", state.id),
            cancellation,
        )
        .and_then(|response| describe_page(area, &url, &response, true, cancellation));
        if cancellation.requested() {
            cancelled = true;
            break;
        }
        let record = match result {
            Ok(description) => {
                let final_url = description
                    .value
                    .get("final_url")
                    .and_then(Value::as_str)
                    .and_then(|url| Url::parse(url).ok());
                let final_in_scope = final_url.as_ref().is_some_and(|final_url| {
                    !state.same_origin
                        || state.seeds.iter().any(|seed| {
                            Url::parse(seed).is_ok_and(|seed| seed.same_origin(final_url))
                        })
                });
                let mut known: HashSet<String> =
                    state.queue.iter().map(|item| item.url.clone()).collect();
                known.extend(state.visited.iter().cloned());
                if let Some(final_url) = &final_url {
                    known.insert(final_url.value.clone());
                }
                let mut discovered = Vec::new();
                if pending.depth < state.max_depth && final_in_scope {
                    for link in description.links {
                        let Ok(link) = Url::parse(&link) else {
                            continue;
                        };
                        let allowed = !state.same_origin
                            || state.seeds.iter().any(|seed| {
                                Url::parse(seed).is_ok_and(|seed| seed.same_origin(&link))
                            });
                        if allowed && known.insert(link.value.clone()) {
                            discovered.push(Pending {
                                url: link.value,
                                depth: pending.depth + 1,
                            });
                        }
                    }
                }
                let mut page = description.value;
                if let Value::Object(fields) = &mut page {
                    fields.insert("depth".into(), Value::number(pending.depth));
                }
                Record { page, discovered }
            }
            Err(error) => Record {
                page: Value::object([
                    ("requested_url", Value::string(&url.value)),
                    ("depth", Value::number(pending.depth)),
                    ("error", Value::string(error)),
                ]),
                discovered: Vec::new(),
            },
        };
        append(area, &state.id, &record)?;
        apply(&mut state, &record)?;
        pages_in_call += 1;
        if fresh.len() < RESULT_PAGES {
            fresh.push(summary(&record.page));
        }
        save(area, &mut state)?;
    }
    let manifest_path = format!("tmp:web/crawls/{}/manifest.jsonl", state.id);
    Ok(Value::object([
        ("crawl_id", Value::string(&state.id)),
        ("manifest_path", Value::string(manifest_path)),
        ("complete", Value::Bool(state.queue.is_empty())),
        ("cancelled", Value::Bool(cancelled)),
        ("pages_total", Value::number(state.pages_total)),
        ("pages_in_call", Value::number(pages_in_call)),
        ("pages_truncated", Value::Bool(pages_in_call > fresh.len())),
        ("manifest_repaired", Value::Bool(manifest_repaired)),
        ("queued", Value::number(state.queue.len())),
        ("pages", Value::Array(fresh)),
    ]))
}

fn summary(page: &Value) -> Value {
    let fields = [
        "requested_url",
        "final_url",
        "status",
        "mime",
        "bytes",
        "title",
        "path",
        "text_path",
        "links_count",
        "depth",
        "error",
    ];
    Value::Object(
        fields
            .into_iter()
            .filter_map(|key| page.get(key).map(|value| (key.into(), value.clone())))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn apply(state: &mut State, record: &Record) -> Result<(), String> {
    let requested = record
        .page
        .get("requested_url")
        .and_then(Value::as_str)
        .ok_or("Invalid crawl manifest page")?;
    if state.queue.front().is_none_or(|item| item.url != requested) {
        return Err("Crawl manifest does not match queued URL".into());
    }
    state.queue.pop_front();
    state.visited.insert(requested.to_string());
    if let Some(final_url) = record.page.get("final_url").and_then(Value::as_str) {
        state.visited.insert(final_url.to_string());
    }
    state.queue.extend(record.discovered.iter().cloned());
    state.pages_total += 1;
    Ok(())
}
