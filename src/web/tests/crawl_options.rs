use super::{
    area,
    http::{Reply, site},
    run,
};
use crate::{cancel::Cancellation, json::Value, web::execute};

fn credentials() -> Value {
    Value::object([
        ("Authorization", Value::string("Bearer isolated-crawl-key")),
        ("Cookie", Value::string("session=fixture")),
        ("X-Private-Key", Value::string("isolated-custom-key")),
    ])
}

#[test]
fn cross_origin_crawl_does_not_forward_seed_headers_to_links_or_redirects() {
    let foreign = site(|_| Reply::page("foreign"));
    let target = foreign.endpoint.clone();
    let seed = site(move |request| match request.path.as_str() {
        "/redirect" => Reply::redirect(302, format!("{target}/redirected")),
        _ => Reply::page(format!(
            "<a href='{target}/linked'>Foreign</a><a href='/redirect'>Redirect</a>"
        )),
    });
    let (_home, _project, area) = area();
    run(
        &area,
        vec![
            ("action", Value::string("crawl")),
            ("urls", Value::Array(vec![Value::string(&seed.endpoint)])),
            ("same_origin", Value::Bool(false)),
            ("headers", credentials()),
        ],
    );
    for request in seed.finish() {
        assert_eq!(
            request.header("Authorization"),
            Some("Bearer isolated-crawl-key")
        );
        assert_eq!(request.header("X-Private-Key"), Some("isolated-custom-key"));
    }
    let requests = foreign.finish();
    assert_eq!(requests.len(), 2);
    for request in requests {
        for header in ["Authorization", "Cookie", "X-Private-Key"] {
            assert_eq!(request.header(header), None, "{request:?}");
        }
        assert_eq!(request.method, "GET");
    }
}

#[test]
fn crawl_resume_preserves_headers_and_rejects_changed_request_options() {
    let server = site(|request| {
        Reply::page(if request.path == "/next" {
            "x".repeat(10001)
        } else {
            "<a href='/next'>Next</a>".into()
        })
    });
    let (_home, _project, area) = area();
    let first = run(
        &area,
        vec![
            ("action", Value::string("crawl")),
            ("urls", Value::Array(vec![Value::string(&server.endpoint)])),
            ("max_pages", Value::number(1)),
            ("headers", credentials()),
            ("max_bytes", Value::number(10000)),
            ("timeout_seconds", Value::number(7)),
        ],
    );
    let id = first.get("crawl_id").and_then(Value::as_str).unwrap();
    for (key, value) in [
        (
            "headers",
            Value::object([("X-Private-Key", Value::string("changed"))]),
        ),
        ("max_bytes", Value::number(3)),
        ("timeout_seconds", Value::number(1)),
    ] {
        let arguments = Value::object([
            ("action", Value::string("crawl")),
            ("crawl_id", Value::string(id)),
            (key, value),
        ]);
        assert!(
            execute(&area, &arguments, &Cancellation::default()).is_err(),
            "changed {key} accepted"
        );
    }
    let resumed = run(
        &area,
        vec![
            ("action", Value::string("crawl")),
            ("crawl_id", Value::string(id)),
        ],
    );
    assert!(
        resumed.get("pages").and_then(Value::as_array).unwrap()[0]
            .get("error")
            .is_some()
    );
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| request.header("Cookie") == Some("session=fixture"))
    );
}

#[test]
fn crawl_uses_get_without_a_body() {
    let (_home, _project, area) = area();
    for extra in [
        ("method", Value::string("POST")),
        ("body", Value::string("payload")),
    ] {
        let arguments = Value::object([
            ("action", Value::string("crawl")),
            (
                "urls",
                Value::Array(vec![Value::string("http://127.0.0.1:1/")]),
            ),
            extra,
        ]);
        assert!(execute(&area, &arguments, &Cancellation::default()).is_err());
    }
}

#[test]
fn headers_are_available_at_each_explicit_seed_origin() {
    let first = site(|_| Reply::page("first"));
    let second = site(|_| Reply::page("second"));
    let (_home, _project, area) = area();
    run(
        &area,
        vec![
            ("action", Value::string("crawl")),
            (
                "urls",
                Value::Array(vec![
                    Value::string(&first.endpoint),
                    Value::string(&second.endpoint),
                ]),
            ),
            ("headers", credentials()),
        ],
    );
    for request in first.finish().into_iter().chain(second.finish()) {
        assert_eq!(request.header("Cookie"), Some("session=fixture"));
    }
}

#[test]
fn a_crawl_snapshot_without_request_options_is_rejected_unchanged() {
    let server = site(|_| Reply::page("unused"));
    let (_home, _project, area) = area();
    let cancelled = Cancellation::default();
    cancelled.cancel();
    let first = execute(
        &area,
        &Value::object([
            ("action", Value::string("crawl")),
            ("urls", Value::Array(vec![Value::string(&server.endpoint)])),
        ]),
        &cancelled,
    )
    .unwrap();
    let id = first.get("crawl_id").and_then(Value::as_str).unwrap();
    let (_, snapshot) = area
        .file(&format!("web/crawls/{id}/state-1.json"), false)
        .unwrap();
    let mut document = crate::json::parse(&std::fs::read_to_string(&snapshot).unwrap()).unwrap();
    let Value::Object(fields) = &mut document else {
        panic!()
    };
    fields.remove("options");
    let saved = document.encode();
    std::fs::write(&snapshot, &saved).unwrap();
    let error = execute(
        &area,
        &Value::object([
            ("action", Value::string("crawl")),
            ("crawl_id", Value::string(id)),
        ]),
        &Cancellation::default(),
    )
    .unwrap_err();
    assert!(
        error.contains("Crawl state was not found or is invalid"),
        "{error}"
    );
    assert_eq!(std::fs::read_to_string(snapshot).unwrap(), saved);
    assert!(server.finish().is_empty());
}
