//! Installed-Chromium checks for viewport, snapshot semantics, logs and waits.
use super::Registry;
use crate::cancel::Cancellation;
use crate::json::Value;
use crate::scratch::Area;
use crate::test_support::Directory;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::{self, JoinHandle};
use std::time::Duration;

const FORM: &str = r#"<!doctype html><title>Form fixture</title>
<label for="name">Nome</label><input id="name" name="name" value="Ada">
<label>Email <input id="email" name="email" type="email"></label>
<input id="search" aria-label="Search site" placeholder="Type here">
<span id="cap">Postal code</span><input id="zip" aria-labelledby="cap">
<label><input id="terms" type="checkbox" checked> Accept terms</label>
<button id="save" type="button" onclick="document.querySelector('#out').textContent='Saved '+document.querySelector('#name').value">Save</button>
<button id="off" disabled>Disabled action</button>
<select id="color" name="color"><option value="r">Red</option><option value="g" selected>Green</option></select>
<p id="out"></p>
<script>console.log('fixture ready',42);console.error('fixture problem');setTimeout(()=>{throw new Error('fixture boom')},0)</script>"#;
const NEXT: &str = "<!doctype html><title>Next fixture</title><p id=\"next\">Second page</p>";

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
                let Ok((mut socket, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut input = Vec::new();
                while !input.ends_with(b"\r\n\r\n") && input.len() < 8192 {
                    let mut byte = [0];
                    match socket.read(&mut byte) {
                        Ok(1) => input.push(byte[0]),
                        _ => break,
                    }
                }
                let request = String::from_utf8_lossy(&input);
                let (status, body) = if request.starts_with("GET /form ") {
                    ("200 OK", FORM)
                } else if request.starts_with("GET /next ") {
                    ("200 OK", NEXT)
                } else {
                    ("404 Not Found", "missing")
                };
                let _ = write!(
                    socket,
                    "HTTP/1.1 {status}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
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
            let _ = worker.join();
        }
    }
}

struct Session {
    registry: Registry,
    area: Area,
    cancel: Cancellation,
    _directory: Directory,
}

impl Session {
    fn new() -> Self {
        let directory = Directory::new();
        let area = Area::new(
            directory.path().join("123456-789"),
            directory.path().into(),
            "123456-789",
        )
        .unwrap();
        Self {
            registry: Registry::default(),
            area,
            cancel: Cancellation::default(),
            _directory: directory,
        }
    }

    fn run(&self, action: &str, fields: Vec<(&str, Value)>) -> Result<Value, String> {
        let mut arguments = Value::object([("action", Value::string(action))]);
        if let Value::Object(entries) = &mut arguments {
            for (key, value) in fields {
                entries.insert(key.into(), value);
            }
        }
        self.registry
            .execute(&self.area, None, &arguments, &self.cancel)
    }

    fn inner_size(&self, tab: &str) -> (usize, usize) {
        let value = self
            .run(
                "evaluate",
                vec![
                    ("tab_id", Value::string(tab)),
                    ("expression", Value::string("[innerWidth,innerHeight]")),
                ],
            )
            .unwrap();
        let size = value.get("value").and_then(Value::as_array).unwrap();
        (size[0].as_usize().unwrap(), size[1].as_usize().unwrap())
    }

    /// PNG dimensions read from the IHDR chunk of a saved screenshot.
    fn png_size(&self, result: &Value) -> (u32, u32) {
        let relative = result
            .get("screenshot_path")
            .and_then(Value::as_str)
            .unwrap()
            .strip_prefix("tmp:")
            .unwrap();
        let bytes = std::fs::read(self.area.ensure().unwrap().join(relative)).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&bytes[12..16], b"IHDR");
        (
            u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
            u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
        )
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.registry.stop_all();
    }
}

fn size(width: usize, height: usize) -> Value {
    Value::object([
        ("width", Value::number(width)),
        ("height", Value::number(height)),
    ])
}

fn tab_of(result: &Value) -> String {
    result.get("tab_id").and_then(Value::as_str).unwrap().into()
}

fn viewport_contract(visible: bool) {
    let site = Site::new();
    let session = Session::new();
    let form = format!("{}form", site.url);
    let opened = session
        .run(
            "open",
            vec![
                ("url", Value::string(&form)),
                ("visible", Value::Bool(visible)),
                ("width", Value::number(1100)),
                ("height", Value::number(760)),
            ],
        )
        .unwrap();
    assert_eq!(opened.get("viewport"), Some(&size(1100, 760)));
    let tab = tab_of(&opened);
    assert_eq!(session.inner_size(&tab), (1100, 760));
    let shot = session
        .run("screenshot", vec![("tab_id", Value::string(&tab))])
        .unwrap();
    assert_eq!(session.png_size(&shot), (1100, 760));
    assert!(shot.get("viewport").is_none());
    session
        .run(
            "navigate",
            vec![
                ("tab_id", Value::string(&tab)),
                ("url", Value::string(format!("{}next", site.url))),
            ],
        )
        .unwrap();
    session
        .run(
            "wait",
            vec![
                ("tab_id", Value::string(&tab)),
                ("selector", Value::string("#next")),
            ],
        )
        .unwrap();
    assert_eq!(session.inner_size(&tab), (1100, 760));
    let resized = session
        .run(
            "screenshot",
            vec![
                ("tab_id", Value::string(&tab)),
                ("width", Value::number(800)),
                ("height", Value::number(600)),
            ],
        )
        .unwrap();
    assert_eq!(resized.get("viewport"), Some(&size(800, 600)));
    assert_eq!(session.png_size(&resized), (800, 600));
    assert_eq!(session.inner_size(&tab), (800, 600));
}

#[test]
#[ignore = "requires an installed Chrome or Edge; isolated local site and temporary profile"]
fn live_chromium_viewport_dimensions_and_validation() {
    viewport_contract(false);
    let site = Site::new();
    let session = Session::new();
    for (fields, message) in [
        (vec![("width", Value::number(1100))], "together"),
        (vec![("height", Value::number(760))], "together"),
        (
            vec![("width", Value::number(0)), ("height", Value::number(760))],
            "from 1 to 10000",
        ),
        (
            vec![
                ("width", Value::number(20000)),
                ("height", Value::number(760)),
            ],
            "from 1 to 10000",
        ),
        (
            vec![
                ("width", Value::string("1100")),
                ("height", Value::number(760)),
            ],
            "from 1 to 10000",
        ),
    ] {
        let error = session.run("open", fields).unwrap_err();
        assert!(error.contains(message), "{error}");
    }
    assert!(
        session
            .run("tabs", vec![])
            .unwrap_err()
            .contains("call open first"),
        "an invalid open must not leave a browser behind"
    );
    let plain = session
        .run(
            "open",
            vec![("url", Value::string(format!("{}next", site.url)))],
        )
        .unwrap();
    assert!(plain.get("viewport").is_none());
    let tab = tab_of(&plain);
    let (width, height) = session.inner_size(&tab);
    assert!(width > 0 && height > 0);
    let error = session
        .run(
            "screenshot",
            vec![
                ("tab_id", Value::string(&tab)),
                ("height", Value::number(600)),
            ],
        )
        .unwrap_err();
    assert!(error.contains("together"), "{error}");
    assert_eq!(session.inner_size(&tab), (width, height));
}

#[test]
#[ignore = "opens a visible Chrome or Edge window; isolated local site and temporary profile"]
fn live_chromium_visible_viewport() {
    viewport_contract(true);
}

fn element<'a>(snapshot: &'a Value, id: &str) -> &'a Value {
    snapshot
        .get("elements")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|element| element.get("id") == Some(&Value::string(id)))
        .unwrap_or_else(|| panic!("element #{id} missing from {}", snapshot.encode()))
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

#[test]
#[ignore = "requires an installed Chrome or Edge; isolated local site and temporary profile"]
fn live_chromium_snapshot_semantics_logs_and_wait_diagnostics() {
    let site = Site::new();
    let session = Session::new();
    let form = format!("{}form", site.url);
    let opened = session
        .run("open", vec![("url", Value::string(&form))])
        .unwrap();
    let tab = tab_of(&opened);
    fn with_tab<'a>(tab: &str, mut fields: Vec<(&'a str, Value)>) -> Vec<(&'a str, Value)> {
        fields.push(("tab_id", Value::string(tab)));
        fields
    }
    let on_tab = |fields| with_tab(&tab, fields);
    session
        .run("wait", on_tab(vec![("selector", Value::string("#out"))]))
        .unwrap();
    let snapshot = session.run("snapshot", on_tab(vec![])).unwrap();
    assert!(text(&snapshot, "text").contains("Nome"));
    let name = element(&snapshot, "name");
    assert_eq!(
        (text(name, "role"), text(name, "name"), text(name, "value")),
        ("textbox", "Nome", "Ada")
    );
    assert_eq!(text(name, "html_name"), "name");
    assert_eq!(text(element(&snapshot, "email"), "name"), "Email");
    let search = element(&snapshot, "search");
    assert_eq!(text(search, "name"), "Search site");
    assert_eq!(text(search, "placeholder"), "Type here");
    assert_eq!(text(element(&snapshot, "zip"), "name"), "Postal code");
    let terms = element(&snapshot, "terms");
    assert_eq!(
        (text(terms, "role"), text(terms, "name")),
        ("checkbox", "Accept terms")
    );
    assert_eq!(terms.get("checked"), Some(&Value::Bool(true)));
    let save = element(&snapshot, "save");
    assert_eq!((text(save, "role"), text(save, "name")), ("button", "Save"));
    assert!(save.get("disabled").is_none());
    let off = element(&snapshot, "off");
    assert_eq!(off.get("disabled"), Some(&Value::Bool(true)));
    let color = element(&snapshot, "color");
    assert_eq!(
        (text(color, "role"), text(color, "value")),
        ("combobox", "g")
    );
    let saved = text(&snapshot, "snapshot_path")
        .strip_prefix("tmp:")
        .unwrap();
    let artifact = std::fs::read_to_string(session.area.ensure().unwrap().join(saved)).unwrap();
    assert!(artifact.contains("\"Postal code\""));

    let name_ref = text(name, "ref").to_string();
    session
        .run(
            "fill",
            on_tab(vec![
                ("selector", Value::string(&name_ref)),
                ("text", Value::string("Grace")),
            ]),
        )
        .unwrap();
    session
        .run(
            "click",
            on_tab(vec![("selector", Value::string(text(save, "ref")))]),
        )
        .unwrap();
    session
        .run(
            "wait",
            on_tab(vec![
                ("selector", Value::string("#out")),
                ("text", Value::string("Saved Grace")),
            ]),
        )
        .unwrap();
    let refreshed = session.run("snapshot", on_tab(vec![])).unwrap();
    assert_eq!(text(element(&refreshed, "name"), "value"), "Grace");

    let logs = session.run("logs", on_tab(vec![])).unwrap();
    let entries = logs.get("entries").and_then(Value::as_array).unwrap();
    let has = |kind: &str, level: &str, needle: &str| {
        entries.iter().any(|entry| {
            text(entry, "kind") == kind
                && text(entry, "level") == level
                && text(entry, "text").contains(needle)
        })
    };
    assert!(
        has("console", "log", "fixture ready 42"),
        "{}",
        logs.encode()
    );
    assert!(
        has("console", "error", "fixture problem"),
        "{}",
        logs.encode()
    );
    assert!(
        has("exception", "error", "fixture boom"),
        "{}",
        logs.encode()
    );
    // Ordinary traffic stays out; failures such as a missing favicon remain.
    assert!(
        entries
            .iter()
            .all(|entry| text(entry, "kind") != "network" || text(entry, "level") == "error"),
        "{}",
        logs.encode()
    );
    let saved = text(&logs, "logs_path").strip_prefix("tmp:").unwrap();
    let raw = std::fs::read_to_string(session.area.ensure().unwrap().join(saved)).unwrap();
    assert!(raw.lines().count() >= entries.len());
    assert!(raw.contains("Runtime.consoleAPICalled"));
    assert!(raw.contains("Network.requestWillBeSent"));
    let cursor = text(&logs, "next_cursor").to_string();

    session
        .run(
            "evaluate",
            on_tab(vec![("expression", Value::string("console.warn('later')"))]),
        )
        .unwrap();
    let later = session
        .run("logs", on_tab(vec![("cursor", Value::string(&cursor))]))
        .unwrap();
    let later_entries = later.get("entries").and_then(Value::as_array).unwrap();
    assert_eq!(later_entries.len(), 1, "{}", later.encode());
    assert_eq!(text(&later_entries[0], "level"), "warning");
    assert_eq!(text(&later_entries[0], "text"), "later");

    let network = session
        .run(
            "logs",
            on_tab(vec![
                ("filter", Value::string("network")),
                ("limit", Value::number(2)),
            ]),
        )
        .unwrap();
    let first = network.get("entries").and_then(Value::as_array).unwrap();
    assert_eq!(first.len(), 2);
    assert_eq!(network.get("more"), Some(&Value::Bool(true)));
    let next = session
        .run(
            "logs",
            on_tab(vec![
                ("filter", Value::string("network")),
                ("limit", Value::number(2)),
                ("cursor", Value::string(text(&network, "next_cursor"))),
            ]),
        )
        .unwrap();
    let second = next.get("entries").and_then(Value::as_array).unwrap();
    let last = first.last().unwrap().get("seq").and_then(Value::as_usize);
    assert!(
        second
            .iter()
            .all(|entry| entry.get("seq").and_then(Value::as_usize) > last)
    );

    let missing = session
        .run(
            "wait",
            on_tab(vec![
                ("text", Value::string("Never shown")),
                ("timeout_ms", Value::number(300)),
            ]),
        )
        .unwrap_err();
    assert!(
        missing.contains("Browser wait timed out after 300 ms"),
        "{missing}"
    );
    assert!(missing.contains(&form), "{missing}");
    assert!(missing.contains(&tab), "{missing}");
    assert!(missing.contains("text \"Never shown\""), "{missing}");
}
