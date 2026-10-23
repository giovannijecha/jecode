use super::*;
use crate::attachments::{MARKER, Prompt};
use crate::json::Value;
use crate::test_support::{Directory, completion, tool_call};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn large_images_survive_native_write_and_avanti_without_replaying_the_write() {
    let project = Directory::new();
    let home = Directory::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/chat/completions", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();

    let mut png_a = crate::attachments::tests::png(4, 3);
    png_a.resize(4 * 1024 * 1024, 0xA5);
    let mut png_b = crate::attachments::tests::png(4, 3);
    png_b.resize(4 * 1024 * 1024, 0xB6);
    let expected = [
        format!(
            "data:image/png;base64,{}",
            crate::attachments::base64::encode(&png_a)
        ),
        format!(
            "data:image/png;base64,{}",
            crate::attachments::base64::encode(&png_b)
        ),
    ];
    let responses = [
        completion(
            "",
            vec![tool_call(
                "create-draft",
                "write",
                Value::object([
                    ("path", Value::string("draft.txt")),
                    ("content", Value::string("created once")),
                ]),
            )],
        ),
        completion("Created.", vec![]),
        completion("Continued.", vec![]),
    ];
    let server = thread::spawn(move || {
        let mut results = Vec::new();
        for response in responses {
            let started = Instant::now();
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            started.elapsed() < Duration::from_secs(30),
                            "missing request"
                        );
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(30)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some((name, value)) = line.split_once(':')
                    && name.eq_ignore_ascii_case("content-length")
                {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut bytes = vec![0; length];
            reader.read_exact(&mut bytes).unwrap();
            let body = crate::json::parse(std::str::from_utf8(&bytes).unwrap()).unwrap();
            let messages = body.get("messages").and_then(Value::as_array).unwrap();
            let urls: Vec<_> = messages
                .iter()
                .flat_map(|message| {
                    message
                        .get("content")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|part| part.get("image_url")?.get("url")?.as_str())
                })
                .collect();
            let exact_images = urls.len() == expected.len()
                && urls
                    .iter()
                    .zip(&expected)
                    .all(|(actual, expected)| *actual == expected);
            results.push((
                length,
                exact_images,
                messages.last().unwrap().encode().contains("avanti"),
            ));
            let response = response.encode();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
        }
        results
    });

    let mut client = OpenRouter::fixture(endpoint);
    client.fixture_inputs(&["text", "image"]);
    let mut agent = Agent::new(client, Tools::new(project.path()).unwrap());
    agent.enable_sessions(home.path()).unwrap();
    let pool = agent.sessions().unwrap().store().attachments();
    let image_a = pool.import_bytes("first.png", &png_a).unwrap();
    let image_b = pool.import_bytes("second.png", &png_b).unwrap();
    let prompt = Prompt::new(
        format!("Inspect {MARKER} and {MARKER}"),
        vec![image_a, image_b],
    );
    let mut answers = Vec::new();
    agent
        .run_turn(prompt, &mut |event| {
            match event {
                Event::Message { text } => answers.push(text),
                Event::Recovering { .. } => return Err("fixture recovery is unexpected".into()),
                _ => {}
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(answers.last().map(String::as_str), Some("Created."));
    let draft = project.path().join("draft.txt");
    assert_eq!(std::fs::read_to_string(&draft).unwrap(), "created once");
    std::fs::write(&draft, "external change").unwrap();
    agent
        .run_turn("avanti", &mut |event| {
            match event {
                Event::Message { text } => answers.push(text),
                Event::Recovering { .. } => return Err("fixture recovery is unexpected".into()),
                _ => {}
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(answers.last().map(String::as_str), Some("Continued."));
    assert_eq!(std::fs::read_to_string(&draft).unwrap(), "external change");

    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 3);
    for (length, exact_images, _) in &requests {
        assert!(*length > 10 * 1024 * 1024);
        assert!(*exact_images);
    }
    assert!(requests[2].2);
}
