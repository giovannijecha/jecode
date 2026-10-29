//! Bounded, opt-in measurements of a long working turn: the idle frame while
//! questions wait, switching and typing in Ask, and collaborator snapshots.
use super::{ask::Panel, display::Display, state::State};
use crate::{ask::Request, events::Event, json, json::Value, redact::Redactor};
use std::{hint::black_box, time::Instant};

fn samples(name: &str, size: usize, mut work: impl FnMut() -> usize) {
    for _ in 0..3 {
        black_box(work());
    }
    let mut times = Vec::new();
    let mut bytes = 0;
    for _ in 0..21 {
        let started = Instant::now();
        bytes = black_box(work());
        times.push(started.elapsed().as_secs_f64() * 1e6);
    }
    times.sort_by(f64::total_cmp);
    println!(
        "SUMMARY,{name},{size},21,{:.3},{:.3},{:.3},{bytes}",
        times[0], times[10], times[20]
    );
}

fn drain(display: &mut Display, state: &State) -> usize {
    let mut bytes = 0;
    for _ in 0..4096 {
        bytes += display
            .update(state, "fixture/model", "fixture directory", Instant::now())
            .len();
        if !display.needs_draw(state, Instant::now()) {
            return bytes;
        }
    }
    panic!("synthetic layout did not complete within 4096 passes");
}

const QUESTIONS: &str = r#"{"questions":[
    {"id":"format","title":"Format","question":"Which format should the export use?",
     "context":"The current exporter produces one file per report, and large reports are split.",
     "options":[{"id":"csv","label":"CSV","description":"Open in a spreadsheet","recommended":true},
       {"id":"json","label":"JSON","description":"Keep nested fields"},
       {"id":"xlsx","label":"XLSX","description":"One sheet per section"}]},
    {"id":"fields","title":"Fields","question":"Which fields should be included?","multiple":true,
     "options":[{"id":"dates","label":"Dates"},{"id":"owners","label":"Owners"},
       {"id":"totals","label":"Totals"},{"id":"notes","label":"Notes"}]},
    {"id":"where","title":"Delivery","question":"Where should exports go?"},
    {"id":"when","title":"Schedule","question":"When should it run?",
     "options":[{"id":"daily","label":"Daily"},{"id":"weekly","label":"Weekly"}]}]}"#;

/// A turn of `tools` finished calls and an `ask_user` call still waiting.
fn waiting(tools: usize) -> (State, Request) {
    let mut state = State {
        width: 120,
        height: 40,
        ..State::default()
    };
    state.begin_tool_turn();
    for index in 0..tools {
        let id = format!("call{index}");
        let (name, arguments, result) = match index % 3 {
            0 => (
                "bash",
                Value::object([("command", Value::string("cargo test --offline"))]),
                Value::object([
                    ("stdout", Value::string("test result: ok\n".repeat(40))),
                    ("exit_code", Value::number(0)),
                ]),
            ),
            1 => (
                "read",
                Value::object([("path", Value::string(format!("src/file{index}.rs")))]),
                Value::object([("content", Value::string("fn main() {}\n".repeat(200)))]),
            ),
            _ => (
                "edit",
                Value::object([("path", Value::string(format!("src/file{index}.rs")))]),
                Value::object([("diff", Value::string("-old\n+new\n".repeat(20)))]),
            ),
        };
        state.event(Event::ToolStarted {
            id: id.clone(),
            name: name.into(),
            arguments,
        });
        state.event(Event::ToolFinished {
            id,
            name: name.into(),
            summary: "complete".into(),
            result,
        });
    }
    let request = Request::parse("ask", &json::parse(QUESTIONS).unwrap()).unwrap();
    state.event(Event::ToolStarted {
        id: "ask".into(),
        name: "ask_user".into(),
        arguments: json::parse(QUESTIONS).unwrap(),
    });
    state.event(Event::UserQuestion {
        request: request.clone(),
    });
    state.ask_panel = Some(Panel::new(&request, None));
    state.question = Some(request.clone());
    (state, request)
}

fn record(number: usize, tools: usize) -> Value {
    let tool = |index: usize| {
        Value::object([
            ("id", Value::string(format!("c{index}"))),
            ("name", Value::string("read")),
            ("state", Value::string("done")),
            ("text", Value::string("complete")),
            (
                "arguments",
                Value::object([("path", Value::string("src/main.rs"))]),
            ),
            (
                "result",
                Value::object([("content", Value::string("fn main() {}\n".repeat(300)))]),
            ),
        ])
    };
    Value::object([
        ("id", Value::string(format!("a{number}"))),
        ("number", Value::number(number)),
        ("state", Value::string("running")),
        ("title", Value::string("Synthetic collaborator")),
        ("tools", Value::Array((0..tools).map(tool).collect())),
    ])
}

#[test]
#[ignore = "synthetic release UI benchmark; no network, personal configuration or terminal writes"]
fn waiting_turn_ask_and_collaborators() {
    println!("META,120x40; 3 warmups; 21 samples; elapsed microseconds; UI work only");
    for tools in [30, 150, 600] {
        let (mut state, request) = waiting(tools);
        let mut display = Display::new((state.width, state.height));
        drain(&mut display, &state);
        samples("idle_frame", tools, || drain(&mut display, &state));
        samples("ask_switch", tools, || {
            let panel = state.ask_panel.as_mut().unwrap();
            panel.switch(&request, true);
            state.changed();
            drain(&mut display, &state)
        });
        samples("ask_type", tools, || {
            let panel = state.ask_panel.as_mut().unwrap();
            panel.step = 2;
            panel.type_text(&request.questions[2], "x");
            state.changed();
            drain(&mut display, &state)
        });
    }
    let redactor = Redactor::new("fixture-secret-value".into());
    for tools in [20, 100, 400] {
        let records: Vec<_> = (1..=4).map(|number| record(number, tools)).collect();
        let previous = records.clone();
        samples("collaborator_poll", tools, || {
            let redacted: Vec<_> = records.iter().map(|value| redactor.value(value)).collect();
            usize::from(redacted != previous)
        });
    }
}

#[test]
#[ignore = "synthetic release UI benchmark; no network, personal configuration or terminal writes"]
fn ask_keys_through_the_app() {
    use super::{App, keys::Decoded, terminal::Key};
    use crate::{
        agent::Agent, backend::openrouter::OpenRouter, test_support::Directory, tools::Tools,
    };
    let home = Directory::new();
    let directory = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1".into()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let mut app = App::new(agent, super::tests::config(&directory), None);
    let (state, _) = waiting(150);
    app.state = state;
    for index in 0..400 {
        app.state
            .history
            .record(format!("Earlier prompt {index} with ordinary words"));
    }
    let mut display = Display::new((app.state.width, app.state.height));
    drain(&mut display, &app.state);
    let tab = || {
        Decoded::Key(Key {
            code: 9,
            modifiers: 0,
            character: 0,
        })
    };
    let typed = || Decoded::Text("x".into());
    let inputs: [(&str, &dyn Fn() -> Decoded); 2] =
        [("app_ask_tab", &tab), ("app_ask_type", &typed)];
    for (name, input) in inputs {
        samples(name, 400, || {
            app.input(input()).unwrap();
            app.capture_input();
            app.persist_input(false);
            drain(&mut display, &app.state)
        });
    }
}

#[test]
#[ignore = "synthetic release UI benchmark; no network, personal configuration or terminal writes"]
fn ask_switch_repaint() {
    let (mut state, request) = waiting(150);
    let mut display = Display::new((state.width, state.height));
    drain(&mut display, &state);
    let mut bytes = Vec::new();
    for forward in [true, true, true, false, false, false] {
        state.ask_panel.as_mut().unwrap().switch(&request, forward);
        state.changed();
        bytes.push(drain(&mut display, &state));
    }
    println!(
        "SUMMARY,ask_switch_bytes,{},{}",
        bytes.iter().sum::<usize>() / bytes.len(),
        bytes
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join("|")
    );
}
