use super::recovery_tests::{action, create};
use super::*;
use crate::{
    attachments::{Attachment, MARKER, Prompt},
    auth,
    backend::{Backend, openai, openrouter::OpenRouter},
    cancel::Cancellation,
    sessions::Store,
    test_support::{Directory, HttpFixture, Response, completion, tool_call},
};
use std::{collections::BTreeSet, fs};

// Check completed, persisted results within the agent tool's normal wait budget.
// Request setup and journal writes are not attachment latency guarantees.
const COMPLETION_TIMEOUT: Duration = Duration::from_secs(60);

fn response(output: Value) -> Response {
    let event = Value::object([
        ("type", Value::string("response.completed")),
        (
            "response",
            Value::object([
                ("status", Value::string("completed")),
                ("output", Value::Array(vec![output])),
            ]),
        ),
    ]);
    Response::Stream(vec![(
        Duration::ZERO,
        format!("data: {}\n\n", event.encode()),
    )])
}

fn read_fixture(provider: &str, path: &str) -> HttpFixture {
    let arguments = Value::object([("path", Value::string(path))]);
    if provider == "openrouter" {
        HttpFixture::new(vec![
            (
                200,
                completion("", vec![tool_call("view", "read", arguments)]),
            ),
            (200, completion("Image reviewed", vec![])),
        ])
    } else {
        HttpFixture::streaming(vec![
            response(Value::object([
                ("type", Value::string("function_call")),
                ("namespace", Value::string("jecode")),
                ("call_id", Value::string("view")),
                ("name", Value::string("read")),
                ("arguments", Value::string(arguments.encode())),
            ])),
            response(Value::object([
                ("type", Value::string("message")),
                ("role", Value::string("assistant")),
                (
                    "content",
                    Value::Array(vec![Value::object([
                        ("type", Value::string("output_text")),
                        ("text", Value::string("Image reviewed")),
                    ])]),
                ),
            ])),
        ])
    }
}

fn agent(provider: &str, endpoint: &str, root: &Path) -> Agent {
    let backend: Box<dyn Backend> = if provider == "openrouter" {
        Box::new(OpenRouter::fixture(endpoint.into()))
    } else {
        Box::new(
            openai::OpenAi::with_api(
                openai::Api::fixture(
                    auth::openai::Session::fixture("isolated-attachment-token"),
                    endpoint.trim_end_matches("/chat/completions"),
                ),
                "fixture-model".into(),
            )
            .unwrap(),
        )
    };
    Agent::with_backend(backend, Tools::new(root).unwrap())
}

#[test]
fn shared_and_isolated_collaborators_read_parent_images_and_send_exact_bytes() {
    for provider in ["openrouter", "openai"] {
        for mode in ["shared", "isolated"] {
            let project = Directory::new();
            let home = Directory::new();
            let pool = Store::new(home.path().into(), project.path())
                .unwrap()
                .attachments();
            let bytes = crate::attachments::tests::png(4, 3);
            let image = pool.import_bytes("reference.png", &bytes).unwrap();
            let fixture = read_fixture(provider, &image.reference());
            let mut parent = agent(provider, &fixture.endpoint, project.path());
            parent.enable_sessions(home.path()).unwrap();
            let mut assignment = create(mode);
            let Value::Object(arguments) = &mut assignment else {
                unreachable!()
            };
            let Value::Array(tasks) = arguments.get_mut("tasks").unwrap() else {
                unreachable!()
            };
            let Value::Object(fields) = &mut tasks[0] else {
                unreachable!()
            };
            fields.insert("context".into(), Value::string(image.reference()));
            parent.agent_tool(&assignment).unwrap();
            let records = parent
                .collaboration()
                .wait(Some("a1"), COMPLETION_TIMEOUT, &Cancellation::default())
                .unwrap();
            assert_eq!(
                records[0].get("state").and_then(Value::as_str),
                Some("returned"),
                "{provider}/{mode}: {records:?}"
            );
            let tools = records[0].get("tools").and_then(Value::as_array).unwrap();
            assert_eq!(tools.len(), 1, "{provider}/{mode}: {records:?}");
            let result = tools[0].get("result").unwrap();
            assert!(
                result.get("error").is_none(),
                "{provider}/{mode}: {result:?}"
            );
            assert_eq!(result.get("view").and_then(Value::as_str), Some("image"));
            assert_eq!(
                std::fs::read(pool.load(&image.id).unwrap().path).unwrap(),
                bytes
            );
            parent.agent_tool(&action("close")).unwrap();
            let requests = fixture.finish();
            let included = format!(
                "data:image/png;base64,{}",
                crate::attachments::base64::encode(&bytes)
            );
            assert!(requests[1].body.encode().contains(&included));
        }
    }
}

#[test]
fn child_imports_survive_close_resume_and_parent_deletion() {
    for mode in ["shared", "isolated"] {
        let project = Directory::new();
        let home = Directory::new();
        let bytes = crate::attachments::tests::png(4, 3);
        fs::write(project.path().join("generated.png"), &bytes).unwrap();
        let fixture = read_fixture("openrouter", "generated.png");
        let mut parent = agent("openrouter", &fixture.endpoint, project.path());
        parent.enable_sessions(home.path()).unwrap();
        parent.agent_tool(&create(mode)).unwrap();
        let records = parent
            .collaboration()
            .wait(Some("a1"), COMPLETION_TIMEOUT, &Cancellation::default())
            .unwrap();
        assert_eq!(
            records[0].get("state").and_then(Value::as_str),
            Some("returned"),
            "{mode}: {records:?}"
        );
        let result = records[0].get("tools").and_then(Value::as_array).unwrap()[0]
            .get("result")
            .unwrap();
        let image = Attachment::parse(result.get("attachment").unwrap()).unwrap();
        let store = parent.sessions().unwrap().store();
        let pool = store.attachments();
        assert_eq!(fs::read(pool.load(&image.id).unwrap().path).unwrap(), bytes);
        parent.agent_tool(&action("close")).unwrap();
        fs::remove_file(project.path().join("generated.png")).unwrap();
        // The parent also owns the reference, so deleting it releases this
        // asset immediately unless the retained child journal protects it.
        parent.messages.lock().unwrap().push(
            Prompt::new(
                format!("Keep the reviewed image {MARKER}"),
                vec![image.clone()],
            )
            .message(),
        );
        parent.save_session().unwrap();
        let id = parent.sessions().unwrap().id();
        drop(parent);

        let mut resumed = agent("openrouter", &fixture.endpoint, project.path());
        resumed.enable_sessions(home.path()).unwrap();
        resumed.resume(&id).unwrap();
        let read = resumed.tools.execute(
            "read",
            &Value::object([("path", Value::string(image.reference()))]).encode(),
        );
        assert_eq!(read.get("view").and_then(Value::as_str), Some("image"));
        let (report, replaced) = resumed
            .delete_session(&id, "fixture/model".into(), crate::effort::Effort::Default)
            .unwrap();
        assert!(replaced);
        assert!(report.retained_collaboration.is_some());
        let orphan = pool.import_bytes("orphan.txt", b"unused").unwrap();
        assert_eq!(
            pool.collect(
                store.attachment_bucket(),
                &BTreeSet::new(),
                &BTreeSet::from([image.id.clone(), orphan.id.clone()]),
            )
            .unwrap(),
            1
        );
        assert_eq!(fs::read(pool.load(&image.id).unwrap().path).unwrap(), bytes);
        assert!(pool.load(&orphan.id).is_err());
        fixture.finish();
    }
}

#[test]
fn deleting_a_child_session_keeps_attachments_referenced_by_the_parent() {
    let project = Directory::new();
    let home = Directory::new();
    let mut parent = agent(
        "openrouter",
        "http://127.0.0.1:9/chat/completions",
        project.path(),
    );
    parent.enable_sessions(home.path()).unwrap();
    let child = parent
        .new_child("a1", project.path(), Mode::Shared)
        .unwrap();
    let pool = parent.sessions().unwrap().store().attachments();
    let attachment = pool.import_bytes("shared.txt", b"shared").unwrap();
    let prompt = Prompt::new(MARKER.to_string(), vec![attachment.clone()]);
    parent.messages.lock().unwrap().push(prompt.message());
    parent.save_session().unwrap();
    child.messages.lock().unwrap().push(prompt.message());
    child.save_session().unwrap();
    let child_store = child.sessions().unwrap().store();
    assert_eq!(
        fs::read(child_store.attachments().load(&attachment.id).unwrap().path).unwrap(),
        b"shared"
    );
    child
        .sessions()
        .unwrap()
        .delete(&BTreeSet::new(), || {})
        .unwrap();
    assert_eq!(
        fs::read(pool.load(&attachment.id).unwrap().path).unwrap(),
        b"shared"
    );
}
