//! Actual processes behind an inert provider; all files belong to isolated fixtures.
use super::tool_tests::{call, calls_response, outputs};
use super::*;
use crate::{
    command::tests as native,
    json::{self, Value},
    providers::openai_account::{Progress, Request, Response, Status},
    tls::Budget,
    workspace_fixture::Fixture as Files,
};
use std::{ops::ControlFlow, sync::Mutex};

struct Backend {
    mode: &'static str,
    extra: bool,
    requests: Arc<Mutex<Vec<String>>>,
}
impl worker::Backend for Backend {
    fn login(
        &mut self,
        _: &Budget<'_>,
        _: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), client::Error> {
        Ok(())
    }
    fn generate(
        &mut self,
        request: &Request,
        _: &Budget<'_>,
        _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error> {
        let mut requests = self.requests.lock().unwrap();
        requests.push(request.encode(2 * 1024 * 1024)?);
        Ok(if requests.len() == 1 {
            let args = json::encode(
                &json::object([
                    ("command", Value::String(native::script(self.mode))),
                    ("timeout_seconds", Value::Number("30".into())),
                ]),
                8192,
            )
            .unwrap();
            let mut calls = vec![call("command", "run_command", &args)];
            if self.mode == "write" {
                calls.push(call(
                    "read",
                    "read_file",
                    r#"{"path":"command-result.txt"}"#,
                ));
            }
            if self.extra {
                calls.push(call(
                    "create",
                    "create_file",
                    r#"{"path":"created.txt","content":"created"}"#,
                ));
                calls.push(call("again", "run_command", &args));
            }
            calls_response(calls)
        } else {
            tests::response("Observed the command result.", Status::Completed)
        })
    }
}
pub(crate) struct Run {
    pub session: Session,
    pub files: Files,
    pub requests: Arc<Mutex<Vec<String>>>,
}
pub(crate) fn start(mode: &'static str, extra: bool) -> Run {
    let files = Files::new();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_backend(
        Model::Luna,
        Backend {
            mode,
            extra,
            requests: requests.clone(),
        },
        Some(crate::workspace::Workspace::open(&files.0).unwrap()),
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Ready));
    Run {
        session,
        files,
        requests,
    }
}
pub(crate) fn next(session: &mut Session) -> Event {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        if let Some(event) = session.poll() {
            return event;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "command worker did not produce an event"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
#[cfg(windows)]
fn configured_powershell7_is_used_for_request_preview_and_execution() {
    let Ok(path) = std::env::var("JECODE_TEST_PWSH") else {
        return;
    };
    let state = crate::state::tests::Fixture::new();
    let store = state.store().unwrap();
    crate::state::settings::Settings::load(&store).unwrap();
    let body = store.read("settings.json", 8192).unwrap().unwrap();
    let Value::Object(mut fields) = json::parse(&body, Default::default()).unwrap() else {
        panic!()
    };
    fields.insert("windows_powershell_executable".into(), Value::String(path));
    store
        .replace(
            "settings.json",
            &json::encode(&Value::Object(fields), 8192).unwrap(),
        )
        .unwrap();
    let settings = crate::state::settings::Settings::load(&store).unwrap();
    let shell =
        crate::command::Shell::configured(settings.windows_powershell_executable.as_deref(), None)
            .unwrap();
    let files = Files::new();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_history_shell(
        Model::Luna,
        Backend {
            mode: "write",
            extra: false,
            requests: requests.clone(),
        },
        Some(crate::workspace::Workspace::open(&files.0).unwrap()),
        history::History::default(),
        shell,
    )
    .unwrap();
    assert!(matches!(next(&mut session), Event::Ready));
    assert!(session.submit("write the fixture"));
    let mut planned = false;
    let mut executed = false;
    loop {
        match next(&mut session) {
            Event::CommandPlanned { preview, .. } => {
                planned = true;
                assert!(preview.shell.contains("PowerShell 7.6.6"));
                assert!(preview.shell.contains("pwsh.exe"));
                assert!(
                    preview
                        .shell
                        .contains("bracketed cwd: supported by session probe")
                );
            }
            Event::CommandFinished { success, .. } => executed = success,
            Event::Finished(_, _) => break,
            _ => {}
        }
    }
    assert!(planned && executed);
    let request = requests.lock().unwrap()[0].clone();
    assert!(request.contains("Command shell: PowerShell 7.6.6"));
    assert!(request.contains("using PowerShell 7.6.6"));
    assert_eq!(
        std::fs::read_to_string(files.0.join("command-result.txt")).unwrap(),
        "one execution\n"
    );
}

#[test]
fn direct_command_runs_once_and_followup_reads_see_its_result() {
    let mut run = start("write", false);
    assert!(
        run.session
            .submit("write fixture through a command and read it")
    );
    let mut output = String::new();
    let mut planned = 0;
    let mut completed = false;
    loop {
        match next(&mut run.session) {
            Event::CommandPlanned { preview, .. } => {
                planned += 1;
                assert_eq!(preview.cwd, ".");
                assert_eq!(preview.timeout_seconds, 30);
                assert!(preview.command.contains("child_fixture"));
            }
            Event::CommandOutput { text, .. } => output.push_str(&text),
            Event::CommandFinished {
                success, failed, ..
            } => {
                assert!(success && !failed);
                completed = true;
            }
            Event::Finished(end, metrics) => {
                assert_eq!(end, End::Complete);
                assert_eq!(metrics.tool_calls, 2);
                assert_eq!(metrics.approval_wait_ms, 0);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(planned, 1);
    assert!(completed && output.contains("written"));
    let receipts = outputs(&run.requests.lock().unwrap()[1]);
    assert_eq!(
        receipts[0].1.get("exit_code").and_then(Value::unsigned),
        Some(0)
    );
    assert_eq!(receipts[0].1.get("executed"), Some(&Value::Bool(true)));
    assert_eq!(
        receipts[1].1.get("text").and_then(Value::text),
        Some("one execution\n")
    );
    run.files
        .write("command-result.txt", "preserved after the turn");
    assert!(run.session.submit("continue without tools"));
    loop {
        if let Event::Finished(End::Complete, _) = next(&mut run.session) {
            break;
        }
    }
    assert_eq!(
        std::fs::read_to_string(run.files.0.join("command-result.txt")).unwrap(),
        "preserved after the turn"
    );
    assert_eq!(outputs(&run.requests.lock().unwrap()[2]), receipts);
}

#[test]
fn multiple_commands_and_file_effects_execute_in_order() {
    let mut run = start("write", true);
    assert!(run.session.submit("do all four operations"));
    loop {
        if let Event::Finished(End::Complete, metrics) = next(&mut run.session) {
            assert_eq!(metrics.tool_calls, 4);
            break;
        }
    }
    let receipts = outputs(&run.requests.lock().unwrap()[1]);
    assert_eq!(
        receipts
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        ["command", "read", "create", "again"]
    );
    assert_eq!(
        receipts[1].1.get("text").and_then(Value::text),
        Some("one execution\n")
    );
    assert_eq!(
        receipts[2].1.get("status").and_then(Value::text),
        Some("applied")
    );
    assert_eq!(receipts[3].1.get("executed"), Some(&Value::Bool(true)));
    assert_eq!(
        std::fs::read_to_string(run.files.0.join("created.txt")).unwrap(),
        "created"
    );
}

#[test]
fn cancellation_before_command_launch_does_not_run_it() {
    let files = Files::new();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    let release = Arc::new(Mutex::new(release_rx));
    let gate: worker::EffectGate = Arc::new(move |name| {
        if name == "run_command" {
            entered_tx.send(()).unwrap();
            release.lock().unwrap().recv().unwrap();
        }
    });
    let history = history::History {
        effect_gate: Some(gate),
        ..Default::default()
    };
    let mut session = Session::with_history(
        Model::Luna,
        Backend {
            mode: "write",
            extra: false,
            requests,
        },
        Some(crate::workspace::Workspace::open(&files.0).unwrap()),
        history,
    )
    .unwrap();
    assert!(matches!(next(&mut session), Event::Ready));
    assert!(session.submit("run"));
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    session.cancel();
    release_tx.send(()).unwrap();
    let mut started = false;
    loop {
        match next(&mut session) {
            Event::CommandStarted { .. } => started = true,
            Event::Finished(End::Failed(Failure::Cancelled), _) => break,
            _ => {}
        }
    }
    assert!(!started);
    assert!(!files.0.join("command-result.txt").exists());
}

#[test]
fn closing_a_running_tree_joins_the_worker() {
    let mut run = start("tree", false);
    assert!(run.session.submit("fixture"));
    let mut output = String::new();
    while !output.contains("stream-ready") {
        match next(&mut run.session) {
            Event::CommandOutput { text, .. } => output.push_str(&text),
            Event::Finished(..) => panic!("command ended before interruption"),
            _ => {}
        }
    }
    drop(run.session);
    assert_eq!(run.requests.lock().unwrap().len(), 1);
    let start = output.find("descendant=").unwrap() + "descendant=".len();
    native::assert_stopped(
        output[start..]
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap(),
    );
}

#[test]
fn cancellation_retains_executed_command_receipt_for_next_turn() {
    let mut run = start("stream", false);
    assert!(run.session.submit("run until interrupted"));
    loop {
        if let Event::CommandOutput { text, .. } = next(&mut run.session)
            && text.contains("stream-ready")
        {
            break;
        }
    }
    run.session.cancel();
    let mut receipt = false;
    loop {
        match next(&mut run.session) {
            Event::CommandFinished {
                success,
                failed,
                summary,
                ..
            } => {
                assert!(!success && failed);
                assert!(summary.contains("interrupted"));
                receipt = true;
            }
            Event::Finished(end, _) => {
                assert_eq!(end, End::Failed(Failure::Cancelled));
                break;
            }
            _ => {}
        }
    }
    assert!(receipt);
    assert!(run.session.submit("report only"));
    loop {
        if let Event::Finished(End::Complete, _) = next(&mut run.session) {
            break;
        }
    }
    let receipts = outputs(&run.requests.lock().unwrap()[1]);
    assert_eq!(
        receipts[0].1.get("status").and_then(Value::text),
        Some("cancelled")
    );
    assert_eq!(receipts[0].1.get("executed"), Some(&Value::Bool(true)));
    assert_eq!(
        receipts[0].1.get("cleanup_confirmed"),
        Some(&Value::Bool(true))
    );
}
