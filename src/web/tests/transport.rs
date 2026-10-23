use super::{
    area,
    http::{Reply, site},
    run,
};
use crate::json::Value;

#[test]
fn head_finishes_without_reading_the_advertised_body() {
    let server = site(|_| Reply::page("x".repeat(1000)));
    let (_home, _project, area) = area();
    let result = run(
        &area,
        vec![
            ("action", Value::string("fetch")),
            ("url", Value::string(&server.endpoint)),
            ("method", Value::string("HEAD")),
        ],
    );
    assert_eq!(result.get("status").and_then(Value::as_usize), Some(200));
    assert!(result.get("preview").is_none());
    assert_eq!(server.finish()[0].method, "HEAD");
}

#[test]
fn post_redirect_switches_to_get_for_301_302_303_and_retains_body_for_307_308() {
    for status in [301, 302, 303, 307, 308] {
        let server = site(move |request| {
            if request.path == "/start" {
                Reply::redirect(status, "/done")
            } else {
                Reply::page("done")
            }
        });
        let (_home, _project, area) = area();
        run(
            &area,
            vec![
                ("action", Value::string("fetch")),
                ("url", Value::string(format!("{}/start", server.endpoint))),
                ("method", Value::string("POST")),
                ("body", Value::string("literal @ bytes")),
                (
                    "headers",
                    Value::object([
                        ("Content-Length", Value::string("15")),
                        ("Content-Type", Value::string("text/plain")),
                    ]),
                ),
            ],
        );
        let requests = server.finish();
        assert_eq!(requests.len(), 2, "{status}: {requests:?}");
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].body, b"literal @ bytes");
        if status <= 303 {
            assert_eq!(requests[1].method, "GET", "{status}");
            assert!(requests[1].body.is_empty());
            assert_eq!(requests[1].header("Content-Type"), None);
        } else {
            assert_eq!(requests[1].method, "POST", "{status}");
            assert_eq!(requests[1].body, b"literal @ bytes");
        }
    }
}

#[test]
fn fetch_redirect_keeps_headers_at_the_requested_origin() {
    let foreign = site(|_| Reply::page("foreign"));
    let target = foreign.endpoint.clone();
    let server = site(move |request| match request.path.as_str() {
        "/start" => Reply::redirect(302, "/local"),
        _ => Reply::redirect(302, format!("{target}/done")),
    });
    let (_home, _project, area) = area();
    run(
        &area,
        vec![
            ("action", Value::string("fetch")),
            ("url", Value::string(format!("{}/start", server.endpoint))),
            (
                "headers",
                Value::object([("X-Private-Key", Value::string("fixture"))]),
            ),
        ],
    );
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| request.header("X-Private-Key") == Some("fixture"))
    );
    assert_eq!(foreign.finish()[0].header("X-Private-Key"), None);
}

#[test]
fn custom_body_method_changes_to_get_on_303_but_not_307() {
    for status in [303, 307] {
        let server = site(move |request| {
            if request.path == "/start" {
                Reply::redirect(status, "/done")
            } else {
                Reply::page("ok")
            }
        });
        let (_home, _project, area) = area();
        run(
            &area,
            vec![
                ("action", Value::string("fetch")),
                ("url", Value::string(format!("{}/start", server.endpoint))),
                ("method", Value::string("PATCH")),
                ("body", Value::string("patch")),
            ],
        );
        let requests = server.finish();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].body, b"patch");
        assert_eq!(
            requests[1].method,
            if status == 303 { "GET" } else { "PATCH" }
        );
        assert_eq!(
            requests[1].body,
            if status == 303 { &b""[..] } else { b"patch" }
        );
    }
}

#[test]
fn empty_post_and_custom_method_reach_the_server() {
    let server = site(|_| Reply::page("ok"));
    let (_home, _project, area) = area();
    for method in ["POST", "PATCH", "GET"] {
        run(
            &area,
            vec![
                ("action", Value::string("fetch")),
                ("url", Value::string(&server.endpoint)),
                ("method", Value::string(method)),
            ],
        );
    }
    assert_eq!(
        server
            .finish()
            .iter()
            .map(|request| request.method.as_str())
            .collect::<Vec<_>>(),
        ["POST", "PATCH", "GET"]
    );
}
