//! Keyboard behavior checked through the public browser actions in Chromium.
use crate::cancel::Cancellation;
use crate::json::Value;
mod session;
use session::Session;

const PAGE: &str = r#"document.body.innerHTML = `
<input id="before"><textarea id="editor"></textarea><input id="after">
<div id="rich" contenteditable="true"></div>
<form><input id="entry"><button>Submit</button></form>`;
window.events = [];
for (const type of ['keydown', 'keypress', 'keyup', 'beforeinput', 'input']) {
  document.addEventListener(type, e => events.push({
    type, key: e.key, code: e.code, keyCode: e.keyCode, trusted: e.isTrusted,
    ctrl: e.ctrlKey, meta: e.metaKey, shift: e.shiftKey, data: e.data,
    inputType: e.inputType
  }));
}
window.submitted = 0;
document.querySelector('form').onsubmit = e => { e.preventDefault(); submitted++; };
true"#;

#[test]
#[ignore = "requires installed Chromium"]
fn live_chromium_press_inserts_printable_text_and_enter() {
    let session = Session::new();
    session.click("#editor");
    for key in ["a", "6", "Enter", "b"] {
        session.press(key, 0);
    }
    assert_eq!(session.value("editor.value"), Value::string("a6\nb"));
    assert_eq!(
        session
            .value("events.filter(e => e.type === 'keydown').map(e => [e.key,e.code,e.keyCode])"),
        crate::json::parse(
            r#"[["a","KeyA",65],["6","Digit6",54],["Enter","Enter",13],["b","KeyB",66]]"#
        )
        .unwrap()
    );
    assert_eq!(
        session.value("events.every(e => e.trusted)"),
        Value::Bool(true)
    );
    assert_eq!(
        session.value("events.filter(e => e.type === 'keypress').length"),
        Value::number(4)
    );
    session.click("#entry");
    session.press("Enter", 0);
    assert_eq!(session.value("submitted"), Value::number(1));
}

#[test]
#[ignore = "requires installed Chromium"]
fn live_chromium_press_shortcuts_and_reverse_tab() {
    let session = Session::new();
    let primary = if cfg!(target_os = "macos") { 4 } else { 2 };
    session.run(
        "fill",
        &[
            ("selector", Value::string("#editor")),
            ("text", Value::string("before")),
        ],
    );
    session.press("a", primary);
    assert_eq!(
        session.value("[editor.selectionStart,editor.selectionEnd]"),
        crate::json::parse("[0,6]").unwrap()
    );
    session.press("Backspace", 0);
    assert_eq!(session.value("editor.value"), Value::string(""));
    session.press("z", primary);
    assert_eq!(session.value("editor.value"), Value::string("before"));
    session.press("z", primary | 8);
    assert_eq!(session.value("editor.value"), Value::string(""));
    session.press("Tab", 8);
    assert_eq!(
        session.value("document.activeElement.id"),
        Value::string("before")
    );
    session.press("Tab", 0);
    assert_eq!(
        session.value("document.activeElement.id"),
        Value::string("editor")
    );
}

#[test]
#[ignore = "requires installed Chromium"]
fn live_chromium_type_runs_keyboard_input_sequences() {
    let session = Session::new();
    session.run(
        "type",
        &[
            ("selector", Value::string("#editor")),
            ("text", Value::string("First 6!\r\nSecond é 🦀")),
        ],
    );
    assert_eq!(
        session.value("editor.value"),
        Value::string("First 6!\nSecond é 🦀")
    );
    session.run("type", &[("text", Value::string("\tNext"))]);
    assert_eq!(
        session.value("document.activeElement.id"),
        Value::string("after")
    );
    assert_eq!(
        session.value("document.querySelector('#after').value"),
        Value::string("Next")
    );
    session.run(
        "type",
        &[
            ("selector", Value::string("#rich")),
            ("text", Value::string("line one\nline two")),
        ],
    );
    assert_eq!(
        session.value("rich.innerText"),
        Value::string("line one\nline two")
    );
    session.run(
        "type",
        &[
            ("selector", Value::string("#entry")),
            ("text", Value::string("send\n")),
        ],
    );
    assert_eq!(session.value("submitted"), Value::number(1));
    assert_eq!(
        session.value("events.every(e => e.trusted)"),
        Value::Bool(true)
    );
    assert_eq!(
        session.value(
            "events.some(e => e.type === 'beforeinput' && e.inputType === 'insertParagraph')"
        ),
        Value::Bool(true)
    );
}

#[test]
#[ignore = "requires installed Chromium"]
fn live_chromium_type_follows_app_keyboard_navigation() {
    let session = Session::new();
    session.value(
        r#"document.body.innerHTML = '<input id="a1"><input id="b1"><input id="a2"><input id="b2">';
      a1.onkeydown = e => { if (e.key === 'Enter') { e.preventDefault(); a2.focus(); } }; true"#,
    );
    session.click("#a1");
    session.run("type", &[("text", Value::string("A1\nA2\tB2"))]);
    assert_eq!(
        session.value("[a1.value,b1.value,a2.value,b2.value,document.activeElement.id]"),
        crate::json::parse(r#"["A1","","A2","B2","b2"]"#).unwrap()
    );
    session.run("press", &[("key", Value::string("Shift+Tab"))]);
    assert_eq!(
        session.value("document.activeElement.id"),
        Value::string("a2")
    );
    session.press("ArrowRight", 0);
    session.run("press", &[("key", Value::string("+"))]);
    assert_eq!(session.value("a2.value"), Value::string("A2+"));
    session.run(
        "press",
        &[("key", Value::string("a")), ("modifiers", Value::number(8))],
    );
    assert_eq!(session.value("a2.value"), Value::string("A2+A"));
}

#[test]
#[ignore = "requires installed Chromium"]
fn live_chromium_app_can_cancel_keyboard_default_actions() {
    let session = Session::new();
    session.run(
        "type",
        &[
            ("selector", Value::string("#editor")),
            ("text", Value::string("keep")),
        ],
    );
    session.value("editor.onkeydown = e => { if (e.ctrlKey || e.metaKey || e.key === 'Enter') e.preventDefault(); }; true");
    let primary = if cfg!(target_os = "macos") {
        "Meta"
    } else {
        "Ctrl"
    };
    for key in ["A", "C", "V", "Z"] {
        session.run(
            "press",
            &[("key", Value::string(format!("{primary}+{key}")))],
        );
    }
    assert_eq!(session.value("events.filter(e => e.type === 'keydown' && (e.ctrl || e.meta)).map(e => e.key).join('')"), Value::string("acvz"));
    assert_eq!(
        session.value("[editor.selectionStart,editor.selectionEnd]"),
        crate::json::parse("[4,4]").unwrap()
    );
    session.press("Enter", 0);
    assert_eq!(session.value("editor.value"), Value::string("keep"));
    session.press("b", 0);
    assert_eq!(session.value("editor.value"), Value::string("keepb"));
}

#[test]
#[ignore = "requires installed Chromium"]
fn live_chromium_type_finishes_long_text_within_the_default_timeout() {
    let session = Session::new();
    let text = "0123456789".repeat(100);
    session.run(
        "type",
        &[
            ("selector", Value::string("#editor")),
            ("text", Value::string(&text)),
        ],
    );
    assert_eq!(session.value("editor.value"), Value::string(&text));
}

#[test]
#[ignore = "requires installed Chromium"]
fn live_chromium_selector_accepts_widgets_that_manage_their_focus() {
    let session = Session::new();
    session.value(
        r#"document.body.innerHTML = `<input id="first">
<div id="redirect" tabindex="0"><input id="inner"></div>
<div id="plain"><input id="nested"></div><div id="text">text</div>
<input id="off" disabled>`;
      redirect.onfocus = () => inner.focus(); true"#,
    );
    // A page that already has focus runs the widget's handler inside focus().
    let typed = |selector: &str, text: &str| {
        session.attempt(
            "type",
            &[
                ("selector", Value::string(selector)),
                ("text", Value::string(text)),
            ],
        )
    };
    typed("#first", "1").unwrap();
    assert_eq!(session.value("document.hasFocus()"), Value::Bool(true));
    typed("#redirect", "2").unwrap();
    assert_eq!(session.value("inner.value"), Value::string("2"));
    session.click("#nested");
    typed("#plain", "3").unwrap();
    assert_eq!(session.value("nested.value"), Value::string("3"));
    // An element that cannot take focus leaves it, and the input, elsewhere.
    for selector in ["#text", "#off", "#missing"] {
        let error = typed(selector, "4").unwrap_err();
        assert!(error.contains("cannot receive focus"), "{error}");
    }
    assert_eq!(session.value("nested.value"), Value::string("3"));
    assert_eq!(
        session.value("document.activeElement.id"),
        Value::string("nested")
    );
}

#[test]
#[ignore = "requires installed Chromium"]
fn live_chromium_keys_operate_controls_and_selection() {
    let session = Session::new();
    session.value(
        r#"document.body.innerHTML = `<button id="button">Go</button>
<input id="check" type="checkbox"><dialog id="modal"><input></dialog>
<textarea id="editor"></textarea><input id="field" value="word">`;
      window.clicks = 0; button.onclick = () => clicks++; true"#,
    );
    let press = |selector: &str, key: &str| {
        session.run(
            "press",
            &[
                ("selector", Value::string(selector)),
                ("key", Value::string(key)),
            ],
        );
    };
    press("#button", "Space");
    press("#button", "Enter");
    assert_eq!(session.value("clicks"), Value::number(2));
    press("#check", "Space");
    assert_eq!(session.value("check.checked"), Value::Bool(true));
    session.value("modal.showModal(); true");
    session.press("Escape", 0);
    assert_eq!(session.value("modal.open"), Value::Bool(false));
    press("#editor", "a");
    session.press("Enter", 8);
    assert_eq!(session.value("editor.value"), Value::string("a\n"));
    session.press("Enter", 2);
    assert_eq!(session.value("editor.value"), Value::string("a\n"));
    session.value("field.focus(); field.setSelectionRange(4, 4); true");
    session.press("ArrowLeft", 8);
    assert_eq!(
        session.value("[field.selectionStart,field.selectionEnd]"),
        crate::json::parse("[3,4]").unwrap()
    );
    session.press("Backspace", 0);
    assert_eq!(session.value("field.value"), Value::string("wor"));
}

#[test]
#[ignore = "requires installed Chromium"]
fn live_chromium_dialog_opened_by_a_key_is_reported_and_can_be_answered() {
    let session = Session::new();
    session.value(
        r#"window.answer = 'none'; editor.onkeydown = e => {
        if (e.key === 'q') { e.preventDefault(); answer = confirm('Discard draft?'); }
      }; true"#,
    );
    let started = std::time::Instant::now();
    let error = session
        .attempt(
            "type",
            &[
                ("selector", Value::string("#editor")),
                ("text", Value::string("abqcd")),
            ],
        )
        .unwrap_err();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "{error}"
    );
    assert!(error.starts_with("2 of 5 keys completed: "), "{error}");
    assert!(error.contains("confirm") && error.contains("Discard draft?"));
    // The session survives: other actions explain the block without waiting.
    let refused = session
        .attempt("evaluate", &[("expression", Value::string("1"))])
        .unwrap_err();
    assert!(refused.contains("Page.handleJavaScriptDialog"), "{refused}");
    session.run(
        "cdp",
        &[
            ("method", Value::string("Page.handleJavaScriptDialog")),
            ("params", Value::object([("accept", Value::Bool(true))])),
        ],
    );
    assert_eq!(session.value("answer"), Value::Bool(true));
    session.run("type", &[("text", Value::string("cd"))]);
    assert_eq!(session.value("editor.value"), Value::string("abcd"));
}

#[test]
#[ignore = "requires installed Chromium"]
fn live_chromium_dialog_closed_elsewhere_unblocks_the_original_connection() {
    let session = Session::new();
    let port = session.registry.browsers.lock().unwrap()[0].port;
    let targets = super::transport::http_get(port, "/json/list").unwrap();
    let url = targets
        .as_array()
        .unwrap()
        .iter()
        .find(|target| target.get("type").and_then(Value::as_str) == Some("page"))
        .unwrap()
        .get("webSocketDebuggerUrl")
        .and_then(Value::as_str)
        .unwrap();
    let mut outside = super::transport::Cdp::connect(url).unwrap();
    outside
        .call(
            "Page.enable",
            Value::object([]),
            &Cancellation::default(),
            std::time::Duration::from_secs(2),
        )
        .unwrap();
    session.value("window.answer = 'none'; editor.onkeydown = e => { e.preventDefault(); answer = confirm('External close?'); }; true");
    let error = session
        .attempt(
            "press",
            &[
                ("selector", Value::string("#editor")),
                ("key", Value::string("q")),
            ],
        )
        .unwrap_err();
    assert!(error.contains("External close?"), "{error}");
    // A browser-side command must remain responsive while the renderer is blocked.
    let version = outside
        .call(
            "Browser.getVersion",
            Value::object([]),
            &Cancellation::default(),
            std::time::Duration::from_secs(2),
        )
        .unwrap();
    assert!(version.get("product").and_then(Value::as_str).is_some());
    // Another controller closes the dialog, as a person can in a visible window.
    outside
        .call(
            "Page.handleJavaScriptDialog",
            Value::object([("accept", Value::Bool(true))]),
            &Cancellation::default(),
            std::time::Duration::from_secs(2),
        )
        .unwrap();
    assert_eq!(session.value("answer"), Value::Bool(true));
}

#[test]
#[ignore = "requires installed Chromium"]
fn live_chromium_interrupted_key_release_reports_input_already_applied() {
    let session = Session::new();
    session.value("editor.onkeyup = e => { if (e.key === 'b') confirm('Release b?'); }; true");
    let error = session
        .attempt(
            "type",
            &[
                ("selector", Value::string("#editor")),
                ("text", Value::string("abc")),
            ],
        )
        .unwrap_err();
    session.run(
        "cdp",
        &[
            ("method", Value::string("Page.handleJavaScriptDialog")),
            ("params", Value::object([("accept", Value::Bool(false))])),
        ],
    );
    // The second key's down event changed the page before its release failed.
    assert_eq!(session.value("editor.value"), Value::string("ab"));
    assert!(error.starts_with("1 of 3 keys completed: "), "{error}");
    assert!(
        error.contains("keyUp") && error.contains("may already have acted"),
        "{error}"
    );
}
