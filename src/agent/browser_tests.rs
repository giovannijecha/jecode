use super::Agent;
use crate::{
    backend::openrouter::OpenRouter,
    cancel::Cancellation,
    json::{self, Value},
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::{self, JoinHandle};
use std::time::Duration;

struct Site {
    url: String,
    stop: Cancellation,
    worker: Option<JoinHandle<()>>,
}

impl Site {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = Cancellation::default();
        let stopping = stop.clone();
        let worker = thread::spawn(move || {
            while !stopping.requested() {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut request = Vec::new();
                        while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
                            let mut byte = [0];
                            match stream.read(&mut byte) {
                                Ok(1) => request.push(byte[0]),
                                _ => break,
                            }
                        }
                        if !request.ends_with(b"\r\n\r\n") {
                            continue;
                        }
                        let body = r#"<!doctype html><html lang="en"><title>Native browser evidence</title>
<style>body{font:20px sans-serif;background:#f5f2eb;padding:40px}button{padding:12px}</style>
<label>Query <input id="query"></label><button id="go">Run</button><p id="result"></p>
<script>document.querySelector('#go').onclick=()=>{const value=document.querySelector('#query').value;document.querySelector('#result').textContent=value;localStorage.setItem('jecode-test',value);console.log('submitted',value);};</script></html>"#;
                        let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        let _ = stopping.wait(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for Site {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

fn call<const N: usize>(id: &str, action: &str, values: [(&str, Value); N]) -> Value {
    let mut arguments =
        std::collections::BTreeMap::from_iter(values.map(|(name, value)| (name.to_owned(), value)));
    arguments.insert("action".into(), Value::string(action));
    tool_call(id, "browser", Value::Object(arguments))
}

fn result(messages: &[Value], id: &str) -> Value {
    let message = messages
        .iter()
        .find(|message| message.get("tool_call_id").and_then(Value::as_str) == Some(id))
        .unwrap();
    json::parse(message.get("content").unwrap().as_str().unwrap()).unwrap()
}

#[test]
#[ignore = "requires an installed Chromium browser; isolated local site and profile only"]
fn native_browser_tool_delivers_images_and_preserves_only_named_profile_state() {
    let site = Site::new();
    let home = Directory::new();
    let project = Directory::new();
    let provider = HttpFixture::with_wait(
        vec![
            (
                200,
                completion(
                    "",
                    vec![
                        call(
                            "open",
                            "open",
                            [
                                ("url", Value::string(&site.url)),
                                ("profile", Value::string("fixture-profile")),
                            ],
                        ),
                        call(
                            "fill",
                            "fill",
                            [
                                ("selector", Value::string("#query")),
                                ("text", Value::string("Native evidence")),
                            ],
                        ),
                        call("click", "click", [("selector", Value::string("#go"))]),
                        call(
                            "wait",
                            "wait",
                            [
                                ("selector", Value::string("#result")),
                                ("text", Value::string("Native evidence")),
                            ],
                        ),
                        call("snapshot", "snapshot", []),
                        call(
                            "screenshot",
                            "screenshot",
                            [
                                ("width", Value::number(1024)),
                                ("height", Value::number(720)),
                            ],
                        ),
                        call("logs", "logs", []),
                        call("stop", "stop", []),
                        call(
                            "reopen",
                            "open",
                            [
                                ("url", Value::string(&site.url)),
                                ("profile", Value::string("fixture-profile")),
                            ],
                        ),
                        call(
                            "remembered",
                            "evaluate",
                            [(
                                "expression",
                                Value::string("localStorage.getItem('jecode-test')"),
                            )],
                        ),
                    ],
                ),
            ),
            (200, completion("Browser verification complete.", vec![])),
        ],
        Duration::from_secs(90),
    );
    let mut client = OpenRouter::fixture(provider.endpoint.clone());
    client.fixture_inputs(&["text", "image"]);
    let mut agent = Agent::new(client, Tools::new(project.path()).unwrap());
    agent.enable_sessions(home.path()).unwrap();
    agent
        .permissions()
        .set_level(crate::permissions::Level::Full)
        .unwrap();
    agent
        .run_turn(
            "Verify the local page and retain this dedicated login profile",
            &mut |_| Ok(()),
        )
        .unwrap();
    let requests = provider.finish();
    assert_eq!(requests.len(), 2);
    let messages = requests[1]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    for id in [
        "open",
        "fill",
        "click",
        "wait",
        "snapshot",
        "screenshot",
        "logs",
        "stop",
        "reopen",
        "remembered",
    ] {
        let result = result(messages, id);
        assert!(result.get("error").is_none(), "{id}: {result:?}");
    }
    assert!(
        result(messages, "snapshot")
            .encode()
            .contains("Native evidence")
    );
    assert_eq!(
        result(messages, "remembered")
            .get("value")
            .and_then(Value::as_str),
        Some("Native evidence")
    );
    let screenshot = result(messages, "screenshot");
    assert_eq!(
        screenshot.get("view").and_then(Value::as_str),
        Some("image")
    );
    assert_eq!(
        screenshot.get("attachment").unwrap().get("width"),
        Some(&Value::number(1024))
    );
    let last = messages
        .last()
        .unwrap()
        .get("content")
        .unwrap()
        .as_array()
        .unwrap();
    assert!(
        last.iter()
            .any(|part| part.get("type").and_then(Value::as_str) == Some("image_url"))
    );
    let reference = screenshot
        .get("reference")
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        !Value::Array(agent.messages.lock().unwrap().clone())
            .encode()
            .contains("base64,")
    );
    let cancellation = Cancellation::default();
    let interrupt = cancellation.clone();
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        interrupt.cancel();
    });
    let interrupted = agent.tools.execute_with_cancel(
        "browser",
        &Value::object([
            ("action", Value::string("evaluate")),
            ("expression", Value::string("new Promise(() => {})")),
        ])
        .encode(),
        &cancellation,
    );
    canceller.join().unwrap();
    assert!(
        interrupted
            .get("error")
            .and_then(Value::as_str)
            .unwrap()
            .contains("cancelled")
    );
    let recovered = agent.tools.execute(
        "browser",
        &Value::object([("action", Value::string("snapshot"))]).encode(),
    );
    assert!(recovered.get("error").is_none(), "{recovered:?}");
    agent.clean_temporary().unwrap();
    assert_eq!(agent.temporary_info().unwrap().files, 0);
    assert!(
        home.path()
            .join("browser/profiles/fixture-profile")
            .is_dir()
    );
    let reread = agent.tools.execute(
        "read",
        &Value::object([("path", Value::string(reference))]).encode(),
    );
    assert_eq!(reread.get("view").and_then(Value::as_str), Some("image"));
    assert_eq!(std::fs::read_dir(project.path()).unwrap().count(), 0);
    let ephemeral = agent.tools.execute(
        "browser",
        &Value::object([
            ("action", Value::string("open")),
            ("url", Value::string(&site.url)),
        ])
        .encode(),
    );
    assert!(ephemeral.get("error").is_none(), "{ephemeral:?}");
    let old_temporary = agent.temporary_info().unwrap().path;
    let id = agent.sessions().unwrap().id();
    let model = agent.model().to_owned();
    agent.delete_session(&id, model, agent.effort()).unwrap();
    assert!(!std::path::Path::new(&old_temporary).exists());
    let stopped = agent.tools.execute(
        "browser",
        &Value::object([("action", Value::string("snapshot"))]).encode(),
    );
    assert!(stopped.get("error").is_some());
    assert!(
        home.path()
            .join("browser/profiles/fixture-profile")
            .is_dir()
    );
}
