//! Project instructions reach the model as anchored snapshots, so the native
//! prompt and earlier requests stay identical prefixes when AGENTS.md changes.

use super::*;
use crate::agent::environment::NO_INSTRUCTIONS;
use crate::backend::openrouter::OpenRouter;
use crate::context::SNAPSHOT_HEADER;
use crate::events::Event;
use crate::json::Value;
use crate::test_support::{
    Directory, HttpFixture, Request, Response, completion, latest_snapshot, snapshot_count,
    tool_call,
};
use crate::tools::Tools;

fn agent(directory: &Directory, fixture: &HttpFixture) -> Agent {
    Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    )
}

fn messages(request: &Request) -> &[Value] {
    request.body.get("messages").unwrap().as_array().unwrap()
}

fn text(message: &Value) -> &str {
    message.get("content").and_then(Value::as_str).unwrap_or("")
}

/// Every request extends the previous one: the earlier messages are an exact prefix.
fn assert_prefix_chain(requests: &[Request]) {
    for (index, pair) in requests.windows(2).enumerate() {
        let (before, after) = (messages(&pair[0]), messages(&pair[1]));
        assert!(
            after.len() > before.len() && after[..before.len()] == *before,
            "request {} does not extend request {index}",
            index + 1
        );
    }
}

fn containing<'a>(request: &'a Request, needle: &str) -> Vec<&'a Value> {
    messages(request)
        .iter()
        .filter(|message| text(message).contains(needle))
        .collect()
}

fn plan_call() -> Value {
    tool_call(
        "plan",
        "todo_list",
        Value::object([
            ("action", Value::string("set")),
            ("title", Value::string("Inspect")),
            (
                "tasks",
                Value::Array(vec![Value::object([
                    ("id", Value::string("t1")),
                    ("title", Value::string("Inspect")),
                    ("status", Value::string("in_progress")),
                ])]),
            ),
        ]),
    )
}

#[test]
fn instruction_changes_keep_earlier_requests_as_exact_prefixes() {
    let directory = Directory::new();
    fs::create_dir(directory.path().join("src")).unwrap();
    let fixture = HttpFixture::new(vec![
        (200, completion("One", vec![])),
        (200, completion("Two", vec![])),
        (200, completion("Created", vec![])),
        (200, completion("", vec![plan_call()])),
        (200, completion("Planned", vec![])),
        (200, completion("Edited", vec![])),
        (200, completion("Removed", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture);
    let original_system = agent.messages.lock().unwrap()[0].clone();
    agent.run_turn("First request", &mut |_| Ok(())).unwrap();
    agent.run_turn("Second request", &mut |_| Ok(())).unwrap();
    let path = directory.path().join("src").join(FILE_NAME);
    fs::write(&path, "Project rules v1").unwrap();
    agent.run_turn("Third request", &mut |_| Ok(())).unwrap();
    agent.run_turn("Plan the work", &mut |_| Ok(())).unwrap();
    fs::write(&path, "Project rules v2").unwrap();
    agent.run_turn("Fifth request", &mut |_| Ok(())).unwrap();
    fs::remove_file(&path).unwrap();
    agent.run_turn("Sixth request", &mut |_| Ok(())).unwrap();

    let requests = fixture.finish();
    assert_eq!(requests.len(), 7);
    assert_prefix_chain(&requests);
    let last = &requests[6];
    let system = text(&messages(last)[0]);
    assert!(!system.contains("Project rules"));
    // Each real change is one complete snapshot at the position first sent;
    // the unchanged turn and its plan update added none.
    assert_eq!(containing(last, "Project rules v1").len(), 1);
    assert_eq!(containing(last, "Project rules v2").len(), 1);
    let snapshots = containing(last, SNAPSHOT_HEADER);
    assert_eq!(snapshots.len(), 3);
    for snapshot in &snapshots {
        assert!(text(snapshot).contains("Explicit user requests take precedence"));
    }
    assert!(text(snapshots[0]).contains("Source: src/AGENTS.md"));
    assert!(text(snapshots[0]).contains("Directory scope: src and its descendants"));
    assert!(text(snapshots[2]).contains(NO_INSTRUCTIONS));
    assert!(!text(snapshots[2]).contains("Project rules"));
    assert_eq!(
        containing(&requests[4], "Current conversation plan").len(),
        1
    );
    // Snapshots do not make unchanged session state repeat either: only the
    // first request's state and the plan update carry it.
    let ordinary = messages(last)
        .iter()
        .filter(|message| text(message).starts_with("Jecode session state"))
        .count();
    assert_eq!(ordinary, 2);
    // Original history keeps no request-only instruction text.
    let history = agent.messages.lock().unwrap();
    assert_eq!(history[0], original_system);
    assert!(
        history
            .iter()
            .all(|message| !message.encode().contains("Project rules"))
    );
}

fn write_skill(home: &Directory, description: &str) {
    let package = home.path().join("skills/audit");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("SKILL.md"),
        format!("---\nname: audit\ndescription: {description}\n---\n\n# Audit\nSkill body text.\n"),
    )
    .unwrap();
}

#[test]
fn catalog_changes_are_snapshots_that_ordinary_state_neither_repeats_nor_withdraws() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("No skills", vec![])),
        (200, completion("One skill", vec![])),
        (200, completion("", vec![plan_call()])),
        (200, completion("Planned", vec![])),
        (200, completion("Described", vec![])),
        (200, completion("Removed", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture);
    agent.enable_sessions(home.path()).unwrap();
    agent.run_turn("First", &mut |_| Ok(())).unwrap();
    write_skill(&home, "Audit a change.");
    agent.run_turn("Second", &mut |_| Ok(())).unwrap();
    agent.run_turn("Plan the work", &mut |_| Ok(())).unwrap();
    write_skill(&home, "Review a project.");
    agent.run_turn("Fourth", &mut |_| Ok(())).unwrap();
    fs::remove_dir_all(home.path().join("skills/audit")).unwrap();
    agent.run_turn("Fifth", &mut |_| Ok(())).unwrap();

    let requests = fixture.finish();
    assert_eq!(requests.len(), 6);
    assert_prefix_chain(&requests);
    assert_eq!(latest_snapshot(&requests[0].body), None);
    let created = latest_snapshot(&requests[1].body).unwrap();
    assert!(created.contains("- audit: Audit a change. (skill:audit)"));
    assert!(created.contains(NO_INSTRUCTIONS));
    // Metadata only: skill bodies stay behind on-demand reads.
    assert!(!created.contains("Skill body text"));
    // The plan update is new ordinary state; the catalog stays current.
    for request in &requests[2..4] {
        assert_eq!(snapshot_count(&request.body), 1);
        assert_eq!(latest_snapshot(&request.body).unwrap(), created);
    }
    assert!(
        crate::test_support::latest_state(&requests[3].body).contains("Current conversation plan")
    );
    assert!(
        latest_snapshot(&requests[4].body)
            .unwrap()
            .contains("Review a project.")
    );
    let removed = latest_snapshot(&requests[5].body).unwrap();
    assert!(removed.contains("Personal skills: none are available now"));
    assert!(!removed.contains("skill:audit"));
    assert_eq!(snapshot_count(&requests[5].body), 3);
}

fn failure(code: usize, message: &str) -> Value {
    Value::object([(
        "error",
        Value::object([
            ("code", Value::number(code)),
            ("message", Value::string(message)),
        ]),
    )])
}

#[test]
fn rejected_and_cancelled_requests_do_not_mark_a_snapshot_delivered() {
    let directory = Directory::new();
    fs::write(directory.path().join(FILE_NAME), "Delivered rules").unwrap();
    let fixture = HttpFixture::streaming(vec![
        Response::Http(
            429,
            vec![("Retry-After".into(), "90".into())],
            failure(429, "Wait"),
        ),
        Response::Json(401, failure(401, "Invalid key")),
        Response::Json(200, completion("Accepted", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture);
    let signal = agent.cancellation.clone();
    let cancelled = agent.run_turn("Cancelled", &mut |event| {
        if matches!(event, Event::Recovering { .. }) {
            signal.cancel();
        }
        Ok(())
    });
    assert!(cancelled.unwrap_err().contains("cancelled"));
    assert!(agent.context.latest_state(true).is_none());
    signal.reset();
    assert!(agent.run_turn("Rejected", &mut |_| Ok(())).is_err());
    assert!(agent.context.latest_state(true).is_none());
    agent.run_turn("Accepted", &mut |_| Ok(())).unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    for request in &requests {
        assert_eq!(snapshot_count(&request.body), 1);
        assert!(
            latest_snapshot(&request.body)
                .unwrap()
                .contains("Delivered rules")
        );
    }
    let anchored = agent.context.latest_state(true).unwrap();
    assert_eq!(anchored.before, agent.messages.lock().unwrap().len() - 1);
}

#[test]
fn pending_and_retained_snapshots_count_toward_the_context_estimate() {
    let plain_directory = Directory::new();
    let directory = Directory::new();
    let rules = "Keep this project convention.\n".repeat(1500);
    fs::write(directory.path().join(FILE_NAME), &rules).unwrap();
    let fixture = HttpFixture::new(vec![(200, completion("Done", vec![]))]);
    let mut plain = agent(&plain_directory, &fixture);
    plain.prepare_turn("Inspect").unwrap();
    let mut agent = agent(&directory, &fixture);
    agent.prepare_turn("Inspect").unwrap();
    let floor = plain.locked_estimate() + agent.context.estimate_bytes(rules.len());
    // Pending: the snapshot is not anchored yet.
    assert!(agent.context.latest_state(true).is_none());
    assert!(agent.locked_estimate() >= floor);
    agent.run_turn("Inspect", &mut |_| Ok(())).unwrap();
    // Retained: anchored in the window and no longer pending.
    assert!(agent.context.latest_state(true).is_some());
    assert!(agent.locked_estimate() >= floor);
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn after_compaction_the_current_snapshot_overrides_rules_a_summary_still_mentions() {
    use crate::context::{AUTO_COMPACT_DEFAULT, threshold};
    const CAPACITY: usize = 60_000;
    let directory = Directory::new();
    let path = directory.path().join(FILE_NAME);
    fs::write(&path, "Removed project rule").unwrap();
    let fixture = HttpFixture::new(vec![
        (200, completion("Started.", vec![])),
        (
            200,
            completion(
                "Goal kept; follow the Removed project rule; next: finish.",
                vec![],
            ),
        ),
        (200, completion("Finished.", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture);
    agent.client.fixture_limits(CAPACITY, Some(4096));
    agent
        .run_turn("Start the long task.", &mut |_| Ok(()))
        .unwrap();
    assert_eq!(agent.context.latest_state(true).unwrap().before, 2);
    let budget = threshold(CAPACITY, AUTO_COMPACT_DEFAULT);
    fill_context(&agent, budget);
    fs::remove_file(&path).unwrap();
    agent.run_turn("Continue.", &mut |_| Ok(())).unwrap();

    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    // In place: the summary request is the cached conversation plus the
    // pending withdrawal, so the summarizer also sees the current rules.
    assert!(
        latest_snapshot(&requests[1].body)
            .unwrap()
            .contains(NO_INSTRUCTIONS)
    );
    assert!(agent.context.from > 2);
    // The anchored snapshot left the window; its record outside the window
    // makes the next request restate current instructions after the summary.
    let next = messages(&requests[2]);
    let summary = next
        .iter()
        .position(|message| text(message).contains("follow the Removed project rule"))
        .unwrap();
    let snapshot = next
        .iter()
        .rposition(|message| text(message).starts_with(SNAPSHOT_HEADER))
        .unwrap();
    assert!(snapshot > summary);
    assert!(text(&next[snapshot]).contains(NO_INSTRUCTIONS));
    assert_eq!(snapshot_count(&requests[2].body), 1);
    let states = &agent.context.states;
    assert!(states[0].is_snapshot() && states[0].before < agent.context.from);
    assert_eq!(
        agent.context.latest_state(true).map(|state| state.before),
        Some(agent.messages.lock().unwrap().len() - 1)
    );
}

#[test]
fn withdrawn_instructions_and_catalog_survive_resume_and_repeated_compaction() {
    use crate::context::{AUTO_COMPACT_DEFAULT, threshold};
    const CAPACITY: usize = 60_000;
    let directory = Directory::new();
    let home = Directory::new();
    let path = directory.path().join(FILE_NAME);
    fs::write(&path, "Withdrawn project convention").unwrap();
    write_skill(&home, "Audit a change.");
    let stale_summary = "Follow the Withdrawn project convention and use skill:audit.";
    let fixture = HttpFixture::new(vec![
        (200, completion("Started.", vec![])),
        (200, completion(stale_summary, vec![])),
        (200, completion("First compaction finished.", vec![])),
        (200, completion("Resumed.", vec![])),
        (200, completion(stale_summary, vec![])),
        (200, completion("Second compaction finished.", vec![])),
    ]);
    let mut first = agent(&directory, &fixture);
    first.enable_sessions(home.path()).unwrap();
    first.client.fixture_limits(CAPACITY, Some(4096));
    first.run_turn("Start.", &mut |_| Ok(())).unwrap();
    fs::remove_file(path).unwrap();
    fs::remove_dir_all(home.path().join("skills/audit")).unwrap();
    let budget = threshold(CAPACITY, AUTO_COMPACT_DEFAULT);
    fill_context(&first, budget);
    first.run_turn("Continue.", &mut |_| Ok(())).unwrap();
    let first_boundary = first.context.from;
    assert!(first_boundary > 2);
    let id = first.sessions().unwrap().id();
    first.save_session().unwrap();
    drop(first);

    let mut resumed = agent(&directory, &fixture);
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    resumed.client.fixture_limits(CAPACITY, Some(4096));
    resumed
        .run_turn("Continue after resume.", &mut |_| Ok(()))
        .unwrap();
    fill_context(&resumed, budget);
    resumed.run_turn("Finish.", &mut |_| Ok(())).unwrap();
    assert!(resumed.context.from > first_boundary);
    let original = resumed.messages.lock().unwrap();
    let restored = crate::context::Context::parse(Some(&resumed.context.value())).unwrap();
    restored.validate(&original).unwrap();
    assert_eq!(restored.states, resumed.context.states);
    drop(original);

    let requests = fixture.finish();
    assert_eq!(requests.len(), 6);
    for request in [&requests[2], &requests[3], &requests[5]] {
        let snapshot = latest_snapshot(&request.body).unwrap();
        assert!(snapshot.contains(NO_INSTRUCTIONS));
        assert!(snapshot.contains("Personal skills: none are available now"));
        assert!(!snapshot.contains("Withdrawn project convention"));
        assert!(!snapshot.contains("skill:audit"));
        assert_eq!(snapshot_count(&request.body), 1);
        let body = messages(request);
        let summary = body
            .iter()
            .position(|message| text(message).contains(stale_summary))
            .unwrap();
        let snapshot = body
            .iter()
            .position(|message| text(message).starts_with(SNAPSHOT_HEADER))
            .unwrap();
        assert!(snapshot > summary);
    }
    // Resuming unchanged sources keeps the already delivered withdrawal at
    // its original position, instead of adding another instruction snapshot.
    let before = messages(&requests[2]);
    let after = messages(&requests[3]);
    assert_eq!(after[..before.len()], *before);
}

fn fill_context(agent: &Agent, budget: usize) {
    let mut index = 0;
    while agent.locked_estimate() < budget {
        agent.messages.lock().unwrap().push(Value::object([
            ("role", Value::string("assistant")),
            (
                "content",
                Value::string(format!("Step {index}: {}", "progress ".repeat(400))),
            ),
        ]));
        index += 1;
    }
}
