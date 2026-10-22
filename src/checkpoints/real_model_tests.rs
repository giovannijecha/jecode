//! Explicitly opted-in live-provider checks. No credential is copied to a fixture.
//! Run only with JECODE_REAL_CHECKPOINT_EVAL=1, an output directory, and --ignored.
use super::{
    identity,
    workspace::{Readiness, Workspace},
};
use crate::agent::Agent;
use crate::config::{Settings, Store};
use crate::events::Event;
use crate::json::Value;
use crate::openrouter::OpenRouter;
use crate::test_support::Directory;
use crate::tools::Tools;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

const REQUESTS: usize = 8;
const DEADLINE: Duration = Duration::from_secs(150);

struct Fixture {
    _home: Directory,
    root: Directory,
    agent: Agent,
    workspace: std::sync::Arc<Workspace>,
}

impl Fixture {
    fn new(settings: &Settings, selective: bool) -> Self {
        let home = Directory::new();
        let root = Directory::new();
        fs::write(root.path().join("work.txt"), b"original fixture bytes\n").unwrap();
        if selective {
            fs::write(root.path().join("padding.bin"), vec![71; 300 << 10]).unwrap();
            let archive = identity::locate(home.path(), root.path(), true)
                .unwrap()
                .unwrap();
            fs::write(
                archive.directory.join("policy.json"),
                r#"{"version":1,"max_file_bytes":1048576,"budget_bytes":1048576}"#,
            )
            .unwrap();
        }
        let mut client = OpenRouter::new(settings.api_key.clone(), settings.model.clone()).unwrap();
        client.set_effort(settings.effort);
        let mut agent = Agent::new(client, Tools::new(root.path()).unwrap());
        agent.set_auto_compact(settings.auto_compact);
        agent.set_routing_preferences(settings.routing.clone());
        agent.set_routing(settings.routing_for(&settings.model));
        agent.enable_sessions(home.path()).unwrap();
        let session = super::session::Session::open(home.path(), root.path());
        let workspace = session.workspace().unwrap().clone();
        agent.use_checkpoints(session);
        let started = Instant::now();
        while workspace.readiness() != Readiness::Ready {
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "baseline failed"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(workspace.selective().is_some(), selective);
        Self {
            _home: home,
            root,
            agent,
            workspace,
        }
    }

    fn before_error(&self) -> String {
        let workspace = &self.workspace;
        let target = workspace
            .mark("milestone", "before fixture error", "fixture", true, false)
            .unwrap();
        fs::write(
            self.root.path().join("work.txt"),
            b"another writer's bytes\n",
        )
        .unwrap();
        workspace.verify().unwrap();
        format!("cp-{target}")
    }
}

#[derive(Default)]
struct Observation {
    waiting: usize,
    calls: Vec<(String, Value)>,
    results: Vec<(String, Value)>,
    messages: Vec<String>,
}

impl Observation {
    fn event(&mut self, event: Event) -> Result<(), String> {
        match event {
            Event::Waiting { .. } => {
                self.waiting += 1;
                if self.waiting > REQUESTS {
                    return Err("live fixture request budget reached".into());
                }
            }
            Event::ToolStarted {
                name, arguments, ..
            } => self.calls.push((name, arguments)),
            Event::ToolFinished { name, result, .. } => self.results.push((name, result)),
            Event::Message { text } => self.messages.push(text),
            _ => {}
        }
        Ok(())
    }

    fn action(&self, action: &str) -> usize {
        self.calls
            .iter()
            .filter(|(name, args)| {
                name == "checkpoint" && args.get("action").and_then(Value::as_str) == Some(action)
            })
            .count()
    }

    fn restored_then_undone_then_restored(&self, target: &str) -> bool {
        let restored: Vec<&Value> = self
            .results
            .iter()
            .filter_map(|(name, result)| {
                (name == "checkpoint"
                    && result
                        .get("restored")
                        .and_then(Value::as_array)
                        .is_some_and(|paths| !paths.is_empty()))
                .then_some(result)
            })
            .collect();
        let undo = restored
            .first()
            .and_then(|result| result.get("undo"))
            .and_then(|undo| undo.get("ref"))
            .and_then(Value::as_str);
        undo.is_some()
            && restored
                .first()
                .and_then(|result| result.get("target"))
                .and_then(Value::as_str)
                == Some(target)
            && restored
                .get(1)
                .and_then(|result| result.get("target"))
                .and_then(Value::as_str)
                == undo
            && restored
                .get(2)
                .and_then(|result| result.get("target"))
                .and_then(Value::as_str)
                == Some(target)
    }
}

fn run(
    fixture: &mut Fixture,
    name: &str,
    prompt: &str,
    settings: &Settings,
    output: &Path,
) -> (Observation, bool) {
    let cancel = fixture.agent.cancellation();
    let (done, receiver) = std::sync::mpsc::channel();
    let watchdog = std::thread::spawn(move || {
        if receiver.recv_timeout(DEADLINE).is_err() {
            cancel.cancel();
        }
    });
    let mut observed = Observation::default();
    let start = Instant::now();
    let result = fixture
        .agent
        .run_turn(prompt, &mut |event| observed.event(event));
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    let _ = done.send(());
    watchdog.join().unwrap();
    let charges = fixture.agent.charges();
    let call = |(name, value): &(String, Value)| {
        Value::object([("name", Value::string(name)), ("value", value.clone())])
    };
    let document = Value::object([
        ("case", Value::string(name)),
        ("model", Value::string(&settings.model)),
        ("effort", Value::string(format!("{:?}", settings.effort))),
        ("wall_ms", Value::number(elapsed)),
        ("completion_requests", Value::number(charges.requests)),
        ("requests_with_reported_cost", Value::number(charges.priced)),
        ("reported_cost_usd", Value::number(charges.cost)),
        ("request_budget", Value::number(REQUESTS)),
        (
            "turn_error",
            result.as_ref().err().map_or(Value::Null, Value::string),
        ),
        (
            "calls",
            Value::Array(observed.calls.iter().map(call).collect()),
        ),
        (
            "results",
            Value::Array(observed.results.iter().map(call).collect()),
        ),
        (
            "messages",
            Value::Array(observed.messages.iter().map(Value::string).collect()),
        ),
    ]);
    let text = document.encode().replace(&settings.api_key, "[REDACTED]");
    fs::write(output.join(format!("{name}.json")), text).unwrap();
    println!(
        "LIVE,{name},requests={},priced={},reported_usd={:.9},wall_ms={elapsed:.3},turn_ok={}",
        charges.requests,
        charges.priced,
        charges.cost,
        result.is_ok()
    );
    (observed, result.is_ok())
}

#[test]
#[ignore = "explicit live-provider evaluation; requires configured key, opt-in and output directory"]
fn live_model_recovers_checkpoints() {
    assert_eq!(
        std::env::var("JECODE_REAL_CHECKPOINT_EVAL").as_deref(),
        Ok("1")
    );
    let output = std::env::var_os("JECODE_REAL_CHECKPOINT_OUT").expect("output directory required");
    let output = std::path::PathBuf::from(output);
    fs::create_dir_all(&output).unwrap();
    let settings = Store::discover()
        .unwrap()
        .load()
        .unwrap()
        .expect("Jecode configuration required");
    let mut outcomes = Vec::new();

    let mut fixture = Fixture::new(&settings, false);
    let target = fixture.before_error();
    let prompt = format!(
        "This is an isolated recovery fixture. work.txt was changed incorrectly after {target}. \
         Restore work.txt from {target} using checkpoint, then undo that restore using its undo.ref, \
         then restore {target} again. Use expected: latest for these three authorized restores. \
         Do not reconstruct or rewrite contents manually. Finish with a short confirmation."
    );
    let (observed, ok) = run(&mut fixture, "restore-undo", &prompt, &settings, &output);
    let correct = ok
        && observed.restored_then_undone_then_restored(&target)
        && fs::read(fixture.root.path().join("work.txt")).unwrap() == b"original fixture bytes\n";
    outcomes.push(("restore-undo", correct));
    drop(fixture);

    let mut fixture = Fixture::new(&settings, true);
    let padding = fs::read(fixture.root.path().join("padding.bin")).unwrap();
    let prompt = "This isolated workspace has selective checkpoint coverage. Before using Bash to \
        change work.txt, explicitly capture that individual file with checkpoint. Then use Bash to \
        replace it with temporary fixture text, and restore its original captured state with \
        checkpoint. expected: latest is authorized for this isolated restore. Leave padding.bin \
        untouched. Do not use write/edit to perform recovery. Finish briefly.";
    let (observed, ok) = run(&mut fixture, "selective-bash", prompt, &settings, &output);
    let first = |name: &str| observed.calls.iter().position(|(called, _)| called == name);
    let capture = observed.calls.iter().position(|(name, args)| {
        name == "checkpoint" && args.get("action").and_then(Value::as_str) == Some("capture")
    });
    let correct = ok
        && capture
            .zip(first("bash"))
            .is_some_and(|(saved, bash)| saved < bash)
        && observed.action("restore") >= 1
        && fs::read(fixture.root.path().join("work.txt")).unwrap() == b"original fixture bytes\n"
        && fs::read(fixture.root.path().join("padding.bin")).unwrap() == padding;
    outcomes.push(("selective-bash", correct));
    drop(fixture);

    let mut fixture = Fixture::new(&settings, false);
    let target = fixture.before_error();
    let prompt = format!(
        "In this isolated concurrency fixture, another writer changed work.txt after {target}. \
         Attempt checkpoint restore of work.txt from {target} with its default conflict checking: \
         omit expected, do not force, and do not write/edit the file. Report the conflict briefly. \
         Preserve the other writer's current content."
    );
    let (observed, ok) = run(&mut fixture, "conflict", &prompt, &settings, &output);
    let conflicts = observed.results.iter().any(|(name, result)| {
        name == "checkpoint"
            && result
                .get("conflicts")
                .and_then(Value::as_array)
                .is_some_and(|items| !items.is_empty())
    });
    let correct = ok
        && observed.action("restore") >= 1
        && conflicts
        && fs::read(fixture.root.path().join("work.txt")).unwrap() == b"another writer's bytes\n";
    outcomes.push(("conflict", correct));
    let checks = Value::Array(
        outcomes
            .iter()
            .map(|(name, correct)| {
                Value::object([
                    ("case", Value::string(*name)),
                    ("byte_exact_and_workflow", Value::Bool(*correct)),
                ])
            })
            .collect(),
    );
    fs::write(output.join("checks.json"), checks.encode()).unwrap();
    assert!(
        outcomes.iter().all(|(_, correct)| *correct),
        "live recovery checks: {outcomes:?}"
    );
}
