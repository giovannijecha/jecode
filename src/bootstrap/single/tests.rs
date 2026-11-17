use super::*;
use crate::{
    backend::openrouter::{Api, OpenRouter},
    cancel::Cancellation,
    effort::Effort,
    json, process,
    sessions::{Handle, Store},
    test_support::{Directory, completion, server},
    tools::Tools,
};
use std::{
    io::{BufRead, BufReader, Read},
    net::TcpStream,
    process::Command,
    sync::mpsc,
    time::Duration,
};

fn read_request(stream: &mut TcpStream) -> Value {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut reader = BufReader::new(stream);
    let mut length = 0;
    loop {
        let mut line = String::new();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        if line == "\r\n" {
            break;
        }
        if let Some((key, value)) = line.split_once(':')
            && key.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().unwrap();
        }
    }
    assert!(length <= 8 * 1024 * 1024);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    json::parse(std::str::from_utf8(&body).unwrap()).unwrap()
}

fn reply(stream: &mut TcpStream, text: &str) {
    let body = completion(text, vec![]).encode();
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    // A broken lifecycle can close the native adapter before the title reply.
    let _ = write!(
        stream,
        "HTTP/1.1 200 Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

#[test]
fn piped_one_shots_persist_a_late_native_title_before_the_result_and_process_exit() {
    for (json, output_error) in [(true, false), (false, false), (true, true)] {
        let home = Directory::new();
        let project = Directory::new();
        let (entered, title_entered) = mpsc::channel();
        let (release, title_released) = mpsc::channel();
        let fixture = server::Server::start(move |listener, stop| {
            let mut handlers = Vec::new();
            let mut title_released = Some(title_released);
            for _ in 0..2 {
                let Ok(mut stream) = server::accept(&listener, &stop) else {
                    break;
                };
                let request = read_request(&mut stream);
                if request.get("tools").is_none() {
                    entered.send(()).unwrap();
                    let released = title_released.take().unwrap();
                    handlers.push(std::thread::spawn(move || {
                        if released.recv_timeout(Duration::from_secs(15)).is_ok() {
                            reply(&mut stream, "Generated native title");
                        }
                        request
                    }));
                } else {
                    reply(&mut stream, "Main answer");
                    handlers.push(std::thread::spawn(move || request));
                }
            }
            handlers
                .into_iter()
                .map(|handler| handler.join().unwrap())
                .collect::<Vec<_>>()
        });
        let store = Store::new(home.path().to_path_buf(), project.path()).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "bootstrap::single::tests::single_process_fixture",
                "--ignored",
                "--nocapture",
                "--format=terse",
            ])
            .env("JECODE_TITLE_HOME", home.path())
            .env("JECODE_TITLE_PROJECT", project.path())
            .env("JECODE_TITLE_ENDPOINT", &fixture.endpoint)
            .env("JECODE_TITLE_JSON", if json { "true" } else { "false" })
            .env(
                "JECODE_TITLE_OUTPUT_ERROR",
                if output_error { "true" } else { "false" },
            )
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("OPENROUTER_MODEL");
        let mut observed = Vec::new();
        let mut released = false;
        let output = process::run_observed(
            &mut command,
            Some(b"First request from stdin".to_vec()),
            Duration::from_secs(30),
            128 * 1024,
            &Cancellation::default(),
            None,
            &mut |bytes| {
                observed.extend_from_slice(bytes);
                if !released && String::from_utf8_lossy(&observed).contains("Main answer") {
                    assert!(!String::from_utf8_lossy(&observed).contains("\"type\":\"result\""));
                    title_entered.recv_timeout(Duration::from_secs(10)).unwrap();
                    let id = store.list().unwrap().sessions[0].id.clone();
                    assert!(
                        Handle::open(store.clone(), &id, crate::redact::Redactor::empty()).is_err()
                    );
                    release.send(()).unwrap();
                    released = true;
                }
                Ok(())
            },
        )
        .unwrap();
        assert!(!output.timed_out && !output.cancelled);
        assert_eq!(
            output.exit_code,
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(released, "{}", String::from_utf8_lossy(&observed));
        let requests = fixture.finish();
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|request| request.encode().contains("First request from stdin"))
        );
        let id = store.list().unwrap().sessions[0].id.clone();
        let (handle, _) = Handle::open(store, &id, crate::redact::Redactor::empty()).unwrap();
        let saved = handle.snapshot();
        assert_eq!(saved.title.as_deref(), Some("Generated native title"));
        let titles = saved
            .events
            .iter()
            .filter(|event| event.get("operation").and_then(Value::as_str) == Some("session_title"))
            .collect::<Vec<_>>();
        assert_eq!(titles.len(), 1);
        assert_eq!(
            titles[0].get("status").and_then(Value::as_str),
            Some("complete")
        );
        assert_eq!(titles[0].get("input_tokens"), Some(&Value::Null));
        assert_eq!(titles[0].get("output_tokens"), Some(&Value::Null));
        if json {
            let values = String::from_utf8_lossy(&observed)
                .lines()
                .filter_map(|line| json::parse(line).ok())
                .collect::<Vec<_>>();
            let result = values
                .iter()
                .find(|value| value.get("type").and_then(Value::as_str) == Some("result"))
                .unwrap();
            assert_eq!(
                result.get("status").and_then(Value::as_str),
                Some(if output_error { "failed" } else { "completed" })
            );
            if output_error {
                assert!(
                    result
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap()
                        .contains("fixture output failed")
                );
            }
            assert_eq!(result.get("effort").and_then(Value::as_str), Some("high"));
            assert_eq!(
                result.get("session_id").and_then(Value::as_str),
                Some(id.as_str())
            );
        }
    }
}

struct Output<W> {
    inner: W,
    fail_once: bool,
}

impl<W: Write> Write for Output<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.fail_once && String::from_utf8_lossy(bytes).contains("\"type\":\"message\"") {
            self.fail_once = false;
            self.inner.write_all(bytes)?;
            self.inner.write_all(b"\n")?;
            self.inner.flush()?;
            return Err(std::io::Error::other("fixture output failed"));
        }
        self.inner.write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[test]
#[ignore = "owned subprocess fixture; launched by the one-shot title test"]
fn single_process_fixture() {
    let home = std::path::PathBuf::from(std::env::var_os("JECODE_TITLE_HOME").unwrap());
    let project = std::path::PathBuf::from(std::env::var_os("JECODE_TITLE_PROJECT").unwrap());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/test-fixtures")
        .canonicalize()
        .unwrap();
    assert!(home.canonicalize().unwrap().starts_with(&root));
    assert!(project.canonicalize().unwrap().starts_with(&root));
    let endpoint = std::env::var("JECODE_TITLE_ENDPOINT").unwrap();
    let client = OpenRouter::with_api(Api::fixture(&endpoint), "fixture/model".into()).unwrap();
    let mut agent = Agent::new(client, Tools::new(&project).unwrap());
    agent.set_effort(Effort::High);
    agent.enable_sessions(&home).unwrap();
    agent.name_sessions();
    let input = crate::bootstrap::input::read(&mut std::io::stdin().lock()).unwrap();
    let prompt = crate::bootstrap::input::compose(None, Some(input)).unwrap();
    let output_error = std::env::var("JECODE_TITLE_OUTPUT_ERROR").unwrap() == "true";
    let result = run(
        &mut agent,
        Prompt::new(prompt, vec![]),
        std::env::var("JECODE_TITLE_JSON").unwrap() == "true",
        &mut Output {
            inner: std::io::stdout().lock(),
            fail_once: output_error,
        },
        &mut std::io::stderr().lock(),
    );
    if output_error {
        assert!(result.unwrap_err().contains("fixture output failed"));
    } else {
        result.unwrap();
    }
}
