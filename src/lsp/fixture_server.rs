//! Child process for LSP integration tests. Compiled by fixture.rs, not linked into Jecode.
use std::io::{Read, Write};

fn main() {
    let marker = std::env::args().nth(1).unwrap();
    std::fs::write(&marker, "started\n").unwrap();
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    while let Some(body) = read(&mut input) {
        let text = String::from_utf8(body).unwrap();
        let method = field(&text, "method").unwrap_or_default();
        if method.is_empty() {
            if id_is(&text, 900) {
                log(&marker, "configuration_reply");
            }
            if id_is(&text, 901) {
                log(&marker, "registration_reply");
            }
        }
        if !method.is_empty() {
            log(&marker, &method);
        }
        let id = number(&text, "id");
        match method.as_str() {
            "initialize" => reply(
                &mut output,
                id,
                r#"{"capabilities":{"definitionProvider":true,"referencesProvider":true,"diagnosticProvider":true,"textDocumentSync":2,"positionEncoding":"utf-16"}}"#,
            ),
            "initialized" => {
                send(
                    &mut output,
                    r#"{"jsonrpc":"2.0","id":900,"method":"workspace/configuration","params":{"items":[{"section":"fixture"}]}}"#,
                );
                send(
                    &mut output,
                    r#"{"jsonrpc":"2.0","id":901,"method":"client/registerCapability","params":{"registrations":[{"id":"watch","method":"workspace/didChangeWatchedFiles","registerOptions":{"watchers":[{"globPattern":"**/*.txt","kind":7}]}}]}}"#,
                );
            }
            "textDocument/definition" | "textDocument/references" => {
                if text.contains("\"character\":5") {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                if text.contains("\"character\":7") {
                    error(&mut output, id, "Fixture request failed");
                    continue;
                }
                let uri = field(&text, "uri").unwrap_or_default();
                reply(
                    &mut output,
                    id,
                    &format!(
                        r#"[{{"uri":"{uri}","range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":3}}}}}}]"#
                    ),
                );
            }
            "textDocument/diagnostic" => reply(
                &mut output,
                id,
                r#"{"kind":"full","resultId":"1","items":[]}"#,
            ),
            "textDocument/didChange" => {
                let uri = field(&text, "uri").unwrap_or_default();
                let version = number(&text, "version").unwrap_or(0);
                let items = if text.contains("changed") {
                    r#"[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}},"severity":1,"source":"fixture","message":"missing name"}]"#
                } else {
                    "[]"
                };
                send(
                    &mut output,
                    &format!(
                        r#"{{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{{"uri":"{uri}","version":{version},"diagnostics":{items}}}}}"#
                    ),
                );
            }
            "textDocument/didOpen" => {
                let uri = field(&text, "uri").unwrap_or_default();
                let version = number(&text, "version").unwrap_or(0);
                if uri.ends_with("sample.rs") && version > 1 {
                    send(
                        &mut output,
                        &format!(
                            r#"{{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{{"uri":"{uri}","version":1,"diagnostics":[{{"range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":1}}}},"message":"late old report"}}]}}}}"#
                        ),
                    );
                }
            }
            "shutdown" => reply(&mut output, id, "null"),
            "exit" => break,
            _ => {}
        }
    }
}

fn read(input: &mut impl Read) -> Option<Vec<u8>> {
    let mut header = Vec::new();
    let mut byte = [0];
    while !header.ends_with(b"\r\n\r\n") {
        input.read_exact(&mut byte).ok()?;
        header.push(byte[0]);
        if header.len() > 8192 {
            return None;
        }
    }
    let text = std::str::from_utf8(&header).ok()?;
    let len: usize = text
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))?
        .trim()
        .parse()
        .ok()?;
    let mut body = vec![0; len];
    input.read_exact(&mut body).ok()?;
    Some(body)
}
fn reply(output: &mut impl Write, id: Option<usize>, result: &str) {
    let Some(id) = id else { return };
    let body = format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{result}}}"#);
    write!(output, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    output.flush().unwrap();
}
fn send(output: &mut impl Write, body: &str) {
    write!(output, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    output.flush().unwrap();
}
fn log(marker: &str, message: &str) {
    use std::fs::OpenOptions;
    let mut log = OpenOptions::new().append(true).open(marker).unwrap();
    writeln!(log, "{message}").unwrap();
}
fn id_is(text: &str, id: usize) -> bool {
    number(text, "id") == Some(id)
}
fn error(output: &mut impl Write, id: Option<usize>, message: &str) {
    let Some(id) = id else { return };
    let body =
        format!(r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32803,"message":"{message}"}}}}"#);
    write!(output, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    output.flush().unwrap();
}
fn field(text: &str, name: &str) -> Option<String> {
    let part = text.split(&format!("\"{name}\":")).nth(1)?;
    let part = part.trim_start().strip_prefix('"')?;
    let end = part.find('"')?;
    Some(part[..end].to_owned())
}
fn number(text: &str, name: &str) -> Option<usize> {
    let part = text.split(&format!("\"{name}\":")).nth(1)?;
    let digits: String = part
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}
