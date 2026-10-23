use super::*;
use crate::test_support::Directory;
use std::fs::OpenOptions;
use std::io::Write;
use std::thread;
use std::time::Duration;

mod crawl_options;
mod http;
mod server;
mod transport;
use server::Server;
fn area() -> (Directory, Directory, Area) {
    let home = Directory::new();
    let project = Directory::new();
    let area = Area::new(
        home.path().join("a/b/c"),
        project.path().to_path_buf(),
        "123-1-0",
    )
    .unwrap();
    (home, project, area)
}

fn run(area: &Area, entries: Vec<(&str, Value)>) -> Value {
    let arguments = Value::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect(),
    );
    execute(area, &arguments, &Cancellation::default()).unwrap()
}

#[test]
fn fetch_redirect_extracts_links_and_retains_raw_html() {
    let server = Server::new();
    let (_home, _project, area) = area();
    let result = run(
        &area,
        vec![
            ("action", Value::string("fetch")),
            ("url", Value::string(format!("{}/redirect", server.base))),
        ],
    );
    assert_eq!(result.get("status").and_then(Value::as_usize), Some(200));
    assert_eq!(
        result.get("final_url").and_then(Value::as_str),
        Some(format!("{}/", server.base).as_str())
    );
    assert_eq!(result.get("title").and_then(Value::as_str), Some("Home"));
    let links = result.get("links").and_then(Value::as_array).unwrap();
    assert_eq!(links.len(), 2);
    assert!(
        result
            .get("preview")
            .and_then(Value::as_str)
            .unwrap()
            .contains("# Welcome")
    );
    let raw = area
        .file(
            result
                .get("path")
                .and_then(Value::as_str)
                .unwrap()
                .trim_start_matches("tmp:"),
            false,
        )
        .unwrap()
        .1;
    assert!(
        fs::read_to_string(raw)
            .unwrap()
            .contains("<title>Home</title>")
    );
}

#[test]
fn binary_download_retains_exact_bytes_and_http_errors_are_results() {
    let server = Server::new();
    let (_home, _project, area) = area();
    let result = run(
        &area,
        vec![
            ("action", Value::string("download")),
            ("url", Value::string(format!("{}/binary", server.base))),
        ],
    );
    let raw = area
        .file(
            result
                .get("path")
                .and_then(Value::as_str)
                .unwrap()
                .trim_start_matches("tmp:"),
            false,
        )
        .unwrap()
        .1;
    assert_eq!(fs::read(raw).unwrap(), [0, 1, 2, 255]);
    assert_eq!(result.get("bytes").and_then(Value::as_usize), Some(4));
    let missing = run(
        &area,
        vec![
            ("action", Value::string("fetch")),
            ("url", Value::string(format!("{}/b", server.base))),
        ],
    );
    assert_eq!(missing.get("status").and_then(Value::as_usize), Some(404));
}

#[test]
fn crawl_resumes_queue_without_duplicate_links() {
    let server = Server::new();
    let (_home, _project, area) = area();
    let first = run(
        &area,
        vec![
            ("action", Value::string("crawl")),
            (
                "urls",
                Value::Array(vec![Value::string(server.base.as_str())]),
            ),
            ("max_pages", Value::number(1)),
        ],
    );
    assert_eq!(first.get("pages_total").and_then(Value::as_usize), Some(1));
    assert_eq!(first.get("queued").and_then(Value::as_usize), Some(2));
    let id = first.get("crawl_id").and_then(Value::as_str).unwrap();
    let second = run(
        &area,
        vec![
            ("action", Value::string("crawl")),
            ("crawl_id", Value::string(id)),
            ("max_pages", Value::number(1)),
        ],
    );
    assert_eq!(second.get("pages_total").and_then(Value::as_usize), Some(2));
    let third = run(
        &area,
        vec![
            ("action", Value::string("crawl")),
            ("crawl_id", Value::string(id)),
        ],
    );
    assert_eq!(third.get("pages_total").and_then(Value::as_usize), Some(3));
    assert_eq!(third.get("complete"), Some(&Value::Bool(true)));
    assert_eq!(
        third.get("pages").and_then(Value::as_array).unwrap()[0]
            .get("status")
            .and_then(Value::as_usize),
        Some(404)
    );
}

#[test]
fn cancellation_preserves_crawl_queue() {
    let server = Server::new();
    let (_home, _project, area) = area();
    let cancellation = Cancellation::default();
    let trigger = cancellation.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        trigger.cancel();
    });
    let arguments = Value::object([
        ("action", Value::string("crawl")),
        (
            "urls",
            Value::Array(vec![Value::string(format!("{}/slow", server.base))]),
        ),
    ]);
    let result = execute(&area, &arguments, &cancellation).unwrap();
    assert_eq!(result.get("cancelled"), Some(&Value::Bool(true)));
    assert_eq!(result.get("queued").and_then(Value::as_usize), Some(1));
    assert_eq!(result.get("pages_total").and_then(Value::as_usize), Some(0));
}

#[test]
fn timeout_stops_a_slow_transfer() {
    let server = Server::new();
    let (_home, _project, area) = area();
    let arguments = Value::object([
        ("action", Value::string("fetch")),
        ("url", Value::string(format!("{}/slow", server.base))),
        ("timeout_seconds", Value::number(1)),
    ]);
    let error = execute(&area, &arguments, &Cancellation::default()).unwrap_err();
    assert!(error.contains("HTTP transfer failed"), "{error}");
}

#[test]
fn crawl_resumes_when_only_one_snapshot_exists() {
    let server = Server::new();
    let (_home, _project, area) = area();
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let arguments = Value::object([
        ("action", Value::string("crawl")),
        (
            "urls",
            Value::Array(vec![Value::string(server.base.as_str())]),
        ),
    ]);
    let first = execute(&area, &arguments, &cancellation).unwrap();
    assert_eq!(first.get("cancelled"), Some(&Value::Bool(true)));
    let id = first.get("crawl_id").and_then(Value::as_str).unwrap();
    let second = run(
        &area,
        vec![
            ("action", Value::string("crawl")),
            ("crawl_id", Value::string(id)),
            ("max_pages", Value::number(1)),
        ],
    );
    assert_eq!(second.get("pages_total").and_then(Value::as_usize), Some(1));
}

#[test]
fn extraction_and_inline_links_are_bounded_while_raw_bytes_remain_complete() {
    let (_home, _project, area) = area();
    let url = Url::parse("http://localhost/page").unwrap();
    let alias = "web/fetches/large.body";
    let (_, path) = area.file(alias, true).unwrap();
    let mut html = String::from("<title>Many</title>");
    for index in 0..150 {
        html.push_str(&format!("<a href='/page/{index}'>Item</a>"));
    }
    html.push_str(&"x".repeat(EXTRACT_BYTES + 1));
    fs::write(&path, html.as_bytes()).unwrap();
    let response = Response {
        final_url: url.clone(),
        status: 200,
        mime: Some("text/html".into()),
        bytes: html.len() as u64,
        fetched_at: 1,
        alias: alias.into(),
        path: path.clone(),
    };
    let result = describe_page(&area, &url, &response, true, &Cancellation::default())
        .unwrap()
        .value;
    assert_eq!(result.get("extraction_truncated"), Some(&Value::Bool(true)));
    assert_eq!(
        result.get("links_count").and_then(Value::as_usize),
        Some(150)
    );
    assert_eq!(
        result.get("links").and_then(Value::as_array).unwrap().len(),
        100
    );
    let links = area
        .file(
            result
                .get("links_path")
                .and_then(Value::as_str)
                .unwrap()
                .trim_start_matches("tmp:"),
            false,
        )
        .unwrap()
        .1;
    assert_eq!(
        crate::json::parse(&fs::read_to_string(links).unwrap())
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        150
    );
    assert_eq!(fs::metadata(path).unwrap().len(), html.len() as u64);
    let download = describe_page(&area, &url, &response, false, &Cancellation::default())
        .unwrap()
        .value;
    assert_eq!(download.get("text_path"), Some(&Value::Null));
    assert!(download.get("preview").is_none());
}

#[test]
fn large_crawl_returns_bounded_summaries_and_resume_does_not_rerequest() {
    let server = Server::new();
    let (_home, _project, area) = area();
    let seeds = (0..30)
        .map(|index| Value::string(format!("{}/?seed={index}", server.base)))
        .collect();
    let first = run(
        &area,
        vec![
            ("action", Value::string("crawl")),
            ("urls", Value::Array(seeds)),
            ("max_depth", Value::number(0)),
            ("max_pages", Value::number(30)),
        ],
    );
    assert_eq!(
        first.get("pages_in_call").and_then(Value::as_usize),
        Some(30)
    );
    assert_eq!(
        first.get("pages").and_then(Value::as_array).unwrap().len(),
        20
    );
    assert_eq!(first.get("pages_truncated"), Some(&Value::Bool(true)));
    let manifest = area
        .file(
            first
                .get("manifest_path")
                .and_then(Value::as_str)
                .unwrap()
                .trim_start_matches("tmp:"),
            false,
        )
        .unwrap()
        .1;
    assert_eq!(fs::read_to_string(manifest).unwrap().lines().count(), 30);
    let id = first.get("crawl_id").and_then(Value::as_str).unwrap();
    let resumed = run(
        &area,
        vec![
            ("action", Value::string("crawl")),
            ("crawl_id", Value::string(id)),
        ],
    );
    assert_eq!(
        resumed.get("pages_in_call").and_then(Value::as_usize),
        Some(0)
    );
    assert_eq!(
        resumed.get("pages_total").and_then(Value::as_usize),
        Some(30)
    );
    assert_eq!(server.requests.lock().unwrap().len(), 30);
}

#[test]
fn completed_manifest_record_replays_after_snapshot_loss() {
    let server = Server::new();
    let (_home, _project, area) = area();
    let first = run(
        &area,
        vec![
            ("action", Value::string("crawl")),
            (
                "urls",
                Value::Array(vec![Value::string(server.base.as_str())]),
            ),
            ("max_pages", Value::number(1)),
        ],
    );
    let id = first.get("crawl_id").and_then(Value::as_str).unwrap();
    let newer_snapshot = area
        .file(&format!("web/crawls/{id}/state-0.json"), false)
        .unwrap()
        .1;
    fs::remove_file(newer_snapshot).unwrap();
    let resumed = run(
        &area,
        vec![
            ("action", Value::string("crawl")),
            ("crawl_id", Value::string(id)),
            ("max_pages", Value::number(1)),
        ],
    );
    assert_eq!(
        resumed.get("pages_total").and_then(Value::as_usize),
        Some(2)
    );
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.iter().filter(|path| *path == "/").count(), 1);
}

#[test]
fn valid_manifest_tail_is_preserved_and_incomplete_tail_is_reported() {
    let server = Server::new();
    let (_home, _project, area) = area();
    let first = run(
        &area,
        vec![
            ("action", Value::string("crawl")),
            (
                "urls",
                Value::Array(vec![Value::string(server.base.as_str())]),
            ),
            ("max_depth", Value::number(0)),
        ],
    );
    let id = first.get("crawl_id").and_then(Value::as_str).unwrap();
    let manifest = area
        .file(
            first
                .get("manifest_path")
                .and_then(Value::as_str)
                .unwrap()
                .trim_start_matches("tmp:"),
            false,
        )
        .unwrap()
        .1;
    let mut content = fs::read(&manifest).unwrap();
    assert_eq!(content.pop(), Some(b'\n'));
    fs::write(&manifest, content).unwrap();
    let resumed = run(
        &area,
        vec![
            ("action", Value::string("crawl")),
            ("crawl_id", Value::string(id)),
        ],
    );
    assert_eq!(resumed.get("manifest_repaired"), Some(&Value::Bool(true)));
    assert_eq!(fs::read_to_string(&manifest).unwrap().lines().count(), 1);
    OpenOptions::new()
        .append(true)
        .open(&manifest)
        .unwrap()
        .write_all(b"{\"page\":")
        .unwrap();
    let resumed = run(
        &area,
        vec![
            ("action", Value::string("crawl")),
            ("crawl_id", Value::string(id)),
        ],
    );
    assert_eq!(resumed.get("manifest_repaired"), Some(&Value::Bool(true)));
    assert_eq!(fs::read_to_string(manifest).unwrap().lines().count(), 1);
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}
