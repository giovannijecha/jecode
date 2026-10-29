use super::Agent;
use crate::test_support::server::{self, Server};
use crate::{
    backend::openrouter::OpenRouter,
    json::{self, Value},
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};
use std::io::{Read, Write};
use std::time::Duration;

#[test]
fn web_tool_acquires_locally_and_returns_provenance_without_provider_web_plugins() {
    let home = Directory::new();
    let project = Directory::new();
    let site = Server::start(move |listener, stop| {
        let mut stream = server::accept(&listener, &stop)?;
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            if stream.read(&mut byte).unwrap() == 0 {
                break;
            }
            request.push(byte[0]);
        }
        let body = "<!doctype html><title>Local evidence</title><h1>Acquired locally</h1><a href='/next'>Next page</a>";
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        Ok::<_, std::io::Error>(String::from_utf8(request).unwrap())
    });
    let url = format!("{}/source?one=1&two=2", site.endpoint);
    let provider = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "acquire",
                    "web",
                    Value::object([
                        ("action", Value::string("fetch")),
                        ("url", Value::string(&url)),
                        ("method", Value::string("X-READ")),
                        (
                            "headers",
                            Value::object([("X_Task_ID", Value::string("fixture"))]),
                        ),
                    ]),
                )],
            ),
        ),
        (200, completion("Local acquisition complete.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(provider.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent
        .run_turn("Read the local source", &mut |_| Ok(()))
        .unwrap();
    let request = site.finish().unwrap();
    assert!(request.starts_with("X-READ /source?one=1&two=2 HTTP/"));
    assert!(request.contains("X_Task_ID: fixture\r\n"));
    assert!(!request.to_ascii_lowercase().contains("authorization:"));
    let requests = provider.finish();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert!(request.body.get("plugins").is_none());
        let catalog = request.body.get("tools").unwrap().as_array().unwrap();
        assert!(catalog.iter().any(|tool| {
            tool.get("function")
                .unwrap()
                .get("name")
                .and_then(Value::as_str)
                == Some("web")
        }));
        assert!(catalog.iter().any(|tool| {
            tool.get("function")
                .unwrap()
                .get("name")
                .and_then(Value::as_str)
                == Some("browser")
        }));
    }
    let messages = requests[1]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    let result = messages
        .iter()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        .unwrap();
    let result = json::parse(result.get("content").unwrap().as_str().unwrap()).unwrap();
    assert!(result.get("error").is_none(), "{result:?}");
    let encoded = result.encode();
    assert!(encoded.contains("Local evidence"));
    assert!(encoded.contains("tmp:web/"));
    assert!(encoded.contains(&url));
    assert_eq!(std::fs::read_dir(project.path()).unwrap().count(), 0);
    assert!(agent.temporary_info().unwrap().files > 0);
    agent.clean_temporary().unwrap();
    assert_eq!(agent.temporary_info().unwrap().files, 0);
}

#[test]
fn unavailable_native_tool_returns_an_error_and_keeps_the_turn_usable() {
    let home = Directory::new();
    let project = Directory::new();
    let provider = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "missing-tab",
                    "browser",
                    Value::object([("action", Value::string("snapshot"))]),
                )],
            ),
        ),
        (200, completion("No browser is open yet.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(provider.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent
        .permissions()
        .set_level(crate::permissions::Level::Full)
        .unwrap();
    agent.run_turn("Inspect the page", &mut |_| Ok(())).unwrap();
    let requests = provider.finish();
    let history = requests[1].body.get("messages").unwrap().encode();
    assert!(history.contains("error"));
    assert_eq!(std::fs::read_dir(project.path()).unwrap().count(), 0);
}
