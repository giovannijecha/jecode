use super::*;
use crate::{
    json::{self, Value},
    providers::openai_account::{Progress, Request, Response, Status, client},
    tls::Budget,
    workspace::{Access, Workspace},
    workspace_fixture::Fixture,
};
use std::{
    ops::ControlFlow,
    sync::{Mutex, mpsc},
    time::Duration,
};

struct Capture {
    requests: Arc<Mutex<Vec<Value>>>,
    first_gate: Option<(mpsc::SyncSender<()>, mpsc::Receiver<()>)>,
}
impl Capture {
    fn new(requests: &Arc<Mutex<Vec<Value>>>) -> Self {
        Self {
            requests: requests.clone(),
            first_gate: None,
        }
    }
}
impl worker::Backend for Capture {
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
        let encoded = request.encode(history::MAX_CONTEXT)?;
        self.requests
            .lock()
            .unwrap()
            .push(json::parse(&encoded, Default::default()).unwrap());
        if request.instructions.starts_with("Summarize") {
            return Ok(tests::handoff_response(
                request,
                "Prior work was completed.",
            ));
        }
        if let Some((entered, release)) = self.first_gate.take() {
            entered.send(()).unwrap();
            release.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        Ok(tests::response("done", Status::Completed))
    }
}
fn ready(session: &mut Session) {
    loop {
        match tests::next(session) {
            Event::Ready => break,
            Event::Restored { .. } => {}
            _ => panic!("unexpected setup event"),
        }
    }
}
fn finish(session: &mut Session) {
    loop {
        if let Event::Finished(end, _) = tests::next(session) {
            assert_eq!(end, End::Complete);
            return;
        }
    }
}
fn instructions(request: &Value) -> &str {
    request.get("instructions").and_then(Value::text).unwrap()
}
fn tools(request: &Value) -> &[Value] {
    request.get("tools").and_then(Value::array).unwrap()
}
fn tool_names(request: &Value) -> Vec<&str> {
    tools(request)
        .iter()
        .map(|tool| tool.get("name").and_then(Value::text).unwrap())
        .collect()
}
fn assert_workspace_contract(request: &Value, profile: Access, shell: &crate::command::Shell) {
    let names = tool_names(request);
    assert_eq!(
        names,
        [
            "list_files",
            "read_file",
            "search_text",
            "create_file",
            "edit_file",
            "run_command"
        ]
    );
    let guidance = instructions(request);
    assert!(guidance.contains(&format!("Available Jecode tools: {}.", names.join(", "))));
    assert!(guidance.contains(&format!("File access: {}.", profile.name())));
    assert!(guidance.contains(&format!("Command shell: {}.", shell.label())));
    assert!(guidance.contains(crate::tools::COMMAND_REACH));
    assert!(guidance.contains("Fetching a known URL, searching the web"));
    assert!(guidance.contains("no dedicated web-search or browser tool"));
    assert!(guidance.contains("no verified image-input capability"));
    assert!(guidance.contains("report attempts, specific blockers and unfinished requirements"));
    assert!(!guidance.contains("You cannot browse the web"));
    let command = tools(request)
        .iter()
        .find(|tool| tool.get("name").and_then(Value::text) == Some("run_command"))
        .unwrap()
        .get("description")
        .and_then(Value::text)
        .unwrap();
    assert!(command.contains(&format!("using {}", shell.label())));
    assert!(command.contains(crate::tools::COMMAND_REACH));
    assert!(
        names
            .iter()
            .all(|name| !matches!(*name, "web_search" | "view_image" | "input_image"))
    );
}

#[test]
fn command_enabled_request_does_not_falsely_rule_out_online_work() {
    let files = Fixture::new();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_backend(
        Model::Luna,
        Capture::new(&requests),
        Some(Workspace::open(&files.0).unwrap()),
    )
    .unwrap();
    ready(&mut session);
    assert!(session.submit("Research the site and improve its design"));
    finish(&mut session);
    let captured = requests.lock().unwrap();
    let request = &captured[0];
    let names = tool_names(request);
    assert!(names.contains(&"run_command"));
    assert!(!instructions(request).contains("You cannot browse the web"));
    assert!(instructions(request).contains("network"));
    assert_workspace_contract(
        request,
        Access::Workspace,
        &crate::command::Shell::default(),
    );
}

#[test]
fn local_access_and_conversation_only_emit_different_tool_contracts() {
    let files = Fixture::new();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut local = Session::with_backend(
        Model::Luna,
        Capture::new(&requests),
        Some(
            Workspace::open(&files.0)
                .unwrap()
                .with_access(Access::Local),
        ),
    )
    .unwrap();
    ready(&mut local);
    assert!(local.submit("Read a local file"));
    finish(&mut local);
    let local_request = requests.lock().unwrap().remove(0);
    assert_workspace_contract(
        &local_request,
        Access::Local,
        &crate::command::Shell::default(),
    );
    assert!(instructions(&local_request).contains("Ordinary local files outside it"));

    let mut conversation =
        Session::with_backend(Model::Luna, Capture::new(&requests), None).unwrap();
    ready(&mut conversation);
    assert!(conversation.submit("Discuss an implementation"));
    finish(&mut conversation);
    let captured = requests.lock().unwrap();
    let request = &captured[0];
    assert!(tools(request).is_empty());
    assert!(
        instructions(request)
            .contains("conversation only: no file, command, web search or image tools")
    );
    assert!(!instructions(request).contains("Available Jecode tools:"));
    assert!(!instructions(request).contains(crate::tools::COMMAND_REACH));
    assert!(!instructions(request).contains("Command shell:"));
}

#[test]
fn queued_guidance_and_model_change_rebuild_the_same_effective_capabilities() {
    let files = Fixture::new();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let mut backend = Capture::new(&requests);
    backend.first_gate = Some((entered_tx, release_rx));
    let mut session = Session::with_backend(
        Model::Luna,
        backend,
        Some(Workspace::open(&files.0).unwrap()),
    )
    .unwrap();
    ready(&mut session);
    assert!(session.submit("First task"));
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(session.enqueue("Also check the available local methods"));
    release_tx.send(()).unwrap();
    finish(&mut session);
    {
        let captured = requests.lock().unwrap();
        assert_eq!(captured.len(), 2);
        for request in captured.iter() {
            assert_workspace_contract(
                request,
                Access::Workspace,
                &crate::command::Shell::default(),
            );
        }
        assert_eq!(instructions(&captured[0]), instructions(&captured[1]));
        let followup_input =
            json::encode(captured[1].get("input").unwrap(), history::MAX_CONTEXT).unwrap();
        assert!(followup_input.contains("Also check the available local methods"));
    }

    let selected = Model::new("fixture-vision-model", Some("high")).unwrap();
    assert!(session.set_model(selected));
    assert!(matches!(tests::next(&mut session), Event::ModelChanged(model) if model == selected));
    assert!(session.submit("Continue with the selected model"));
    finish(&mut session);
    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 3);
    assert_eq!(
        captured[2].get("model").and_then(Value::text),
        Some("fixture-vision-model")
    );
    assert_eq!(
        captured[2]
            .get("reasoning")
            .and_then(|v| v.get("effort"))
            .and_then(Value::text),
        Some("high")
    );
    assert_eq!(instructions(&captured[0]), instructions(&captured[2]));
    assert_eq!(tool_names(&captured[0]), tool_names(&captured[2]));
    assert_workspace_contract(
        &captured[2],
        Access::Workspace,
        &crate::command::Shell::default(),
    );
}

#[test]
fn resumed_session_reconstructs_capabilities_without_replaying_history() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let files = Fixture::new();
    let workspace = Workspace::open(&files.0).unwrap();
    let history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    let first = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_history_shell(
        Model::Luna,
        Capture::new(&first),
        Some(workspace),
        history,
        crate::command::Shell::default(),
    )
    .unwrap();
    ready(&mut session);
    assert!(session.submit("Remember this result"));
    finish(&mut session);
    drop(session);

    let saved = persistence::load(&store, &id, true).unwrap();
    let second = Arc::new(Mutex::new(Vec::new()));
    let workspace = Workspace::open(&files.0).unwrap().with_access(saved.access);
    let mut resumed = Session::with_history_shell(
        saved.model,
        Capture::new(&second),
        Some(workspace),
        saved.history,
        crate::command::Shell::default(),
    )
    .unwrap();
    ready(&mut resumed);
    assert!(second.lock().unwrap().is_empty());
    assert!(resumed.submit("Continue without repeating work"));
    finish(&mut resumed);
    let original = first.lock().unwrap();
    let continued = second.lock().unwrap();
    assert_eq!(continued.len(), 1);
    assert_eq!(instructions(&original[0]), instructions(&continued[0]));
    assert_eq!(tool_names(&original[0]), tool_names(&continued[0]));
    assert_workspace_contract(
        &continued[0],
        Access::Workspace,
        &crate::command::Shell::default(),
    );
    let input = json::encode(continued[0].get("input").unwrap(), history::MAX_CONTEXT).unwrap();
    assert!(input.contains("Remember this result"));
    assert!(input.contains("Continue without repeating work"));
}

#[test]
fn compaction_remains_a_non_executing_request_in_a_tool_enabled_session() {
    let files = Fixture::new();
    let mut history = history::History::default();
    history.projection.limit_bytes = 65_536;
    for index in 0..4 {
        history.begin(format!("task-{index}")).unwrap();
        let answer = "a".repeat(28_000);
        let turn = history.turns.last_mut().unwrap();
        turn.steps.push(history::Step {
            text: answer.clone(),
            response: Some(tests::response(&answer, Status::Completed)),
            accepted: true,
            ..Default::default()
        });
        turn.end = Some(End::Complete);
        turn.outcome = "Complete".into();
    }
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_history_shell(
        Model::Luna,
        Capture::new(&requests),
        Some(Workspace::open(&files.0).unwrap()),
        history,
        crate::command::Shell::default(),
    )
    .unwrap();
    ready(&mut session);
    assert!(session.compact());
    finish(&mut session);
    assert!(session.submit("Continue after summary"));
    finish(&mut session);
    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 2);
    assert!(instructions(&captured[0]).starts_with("Summarize"));
    assert!(instructions(&captured[0]).contains("Do not execute tools"));
    assert!(tools(&captured[0]).is_empty());
    assert_workspace_contract(
        &captured[1],
        Access::Workspace,
        &crate::command::Shell::default(),
    );
}

#[cfg(windows)]
#[test]
fn selected_powershell_version_is_in_the_prompt_and_command_schema() {
    let Ok(path) = std::env::var("JECODE_TEST_PWSH") else {
        return;
    };
    let files = Fixture::new();
    let shell = crate::command::Shell::configured(Some(&path), Some(&files.0)).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_history_shell(
        Model::Luna,
        Capture::new(&requests),
        Some(Workspace::open(&files.0).unwrap()),
        history::History::default(),
        shell.clone(),
    )
    .unwrap();
    ready(&mut session);
    assert!(session.submit("Describe available commands"));
    finish(&mut session);
    let captured = requests.lock().unwrap();
    assert_workspace_contract(&captured[0], Access::Workspace, &shell);
    assert!(!instructions(&captured[0]).contains("Windows PowerShell 5.1"));
    let command = tools(&captured[0])
        .iter()
        .find(|tool| tool.get("name").and_then(Value::text) == Some("run_command"))
        .unwrap()
        .get("description")
        .and_then(Value::text)
        .unwrap();
    assert!(command.contains("On Windows PowerShell, a GUI executable"));
}
