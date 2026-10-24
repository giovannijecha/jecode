use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};
use std::io::{Read, Write};
use std::time::{Duration, Instant};

#[test]
fn independent_web_requests_overlap_and_keep_call_order() {
    let site = crate::test_support::server::Server::start(|listener, stop| {
        let mut requests = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        // Neither response is sent until both requests arrive. Sequential
        // execution cannot pass this barrier, regardless of connection speed.
        for _ in 0..2 {
            let mut stream = loop {
                if let Ok((stream, _)) = listener.accept() {
                    break stream;
                }
                assert!(
                    !stop.requested() && Instant::now() < deadline,
                    "web requests did not overlap"
                );
                std::thread::sleep(Duration::from_millis(5));
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                assert_eq!(stream.read(&mut byte).unwrap(), 1);
                request.push(byte[0]);
            }
            requests.push((stream, String::from_utf8(request).unwrap()));
        }
        for (stream, request) in requests.iter_mut().rev() {
            let body = if request.contains("/first ") {
                "First page"
            } else {
                "Second page"
            };
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    let home = Directory::new();
    let project = Directory::new();
    let calls = [("first", "fetch"), ("second", "download")]
        .into_iter()
        .map(|(id, action)| {
            tool_call(
                id,
                "web",
                Value::object([
                    ("action", Value::string(action)),
                    ("url", Value::string(format!("{}/{id}", site.endpoint))),
                ]),
            )
        })
        .collect();
    let provider = HttpFixture::new(vec![
        (200, completion("", calls)),
        (200, completion("Done.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(provider.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let mut events = Vec::new();
    agent
        .run_turn("Fetch both independent pages", &mut |event| {
            match event {
                Event::ToolStarted { id, .. } => events.push(format!("start:{id}")),
                Event::ToolFinished { id, .. } => events.push(format!("finish:{id}")),
                _ => {}
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(
        events,
        [
            "start:first",
            "start:second",
            "finish:first",
            "finish:second"
        ]
    );
    let saved = agent.sessions().unwrap().snapshot();
    assert!(!saved.pending.active && saved.pending.reads.is_empty());
    for (message, id) in saved.messages[3..5].iter().zip(["first", "second"]) {
        assert_eq!(
            message.get("tool_call_id").and_then(Value::as_str),
            Some(id)
        );
        let result = json::parse(message.get("content").and_then(Value::as_str).unwrap()).unwrap();
        assert!(result.get("error").is_none(), "{result:?}");
        assert_eq!(result.get("status"), Some(&Value::number(200)));
    }
    assert_eq!(provider.finish().len(), 2);
    site.finish();
}
