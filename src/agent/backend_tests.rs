use super::*;
use crate::{
    backend::{Limits, Usage},
    conversation,
    test_support::Directory,
};
use std::fs;
pub(super) mod fixture;
use fixture::{Account, Reply};

#[test]
fn account_only_switch_resets_context_and_excludes_foreign_native_items() {
    let directory = Directory::new();
    let home = Directory::new();
    let mut first = Account::new("alpha", vec![Reply::text("First account answer")]);
    first.scope = Some("client-a:subject".into());
    let mut second = Account::new("alpha", vec![Reply::text("Second account answer")]);
    second.scope = Some("client-b:subject".into());
    let mut agent = Agent::new(first.client(), Tools::new(directory.path()).unwrap());
    agent.enable_sessions(home.path()).unwrap();
    agent.run_turn("Original task", &mut |_| Ok(())).unwrap();
    assert_eq!(agent.context.input_tokens, Some(73));
    let original = agent.messages.lock().unwrap().clone();
    agent.replace_backend(Box::new(second.client())).unwrap();
    assert_eq!(agent.context.input_tokens, None);
    assert_eq!(agent.compatible_from, original.len());
    assert_eq!(*agent.messages.lock().unwrap(), original);
    agent
        .run_turn("Continue the same task", &mut |_| Ok(()))
        .unwrap();
    let request = second.requests.lock().unwrap()[0].encode();
    assert!(request.contains("Original task"));
    assert!(!request.contains("alpha-private-state"));
    assert_eq!(
        agent.identity().account.as_deref(),
        Some("client-b:subject")
    );
}

#[test]
fn different_protocols_share_a_session_without_replaying_tools_or_foreign_native_fields() {
    let directory = Directory::new();
    let home = Directory::new();
    let alpha = Account::new(
        "alpha",
        vec![
            Reply {
                text: "Creating the file",
                calls: vec![ToolCall {
                    id: "write-once".into(),
                    name: "write".into(),
                    arguments: Value::object([
                        ("path", Value::string("shared.txt")),
                        ("content", Value::string("original")),
                    ])
                    .encode(),
                }],
            },
            Reply::text("File created"),
            Reply::text("Continuing on alpha"),
        ],
    );
    let beta = Account::new(
        "beta",
        vec![
            Reply::text("Continuing on beta"),
            Reply::text("Task and original file preserved"),
        ],
    );
    let mut agent = Agent::new(alpha.client(), Tools::new(directory.path()).unwrap());
    agent.register_backend(beta.services());
    agent.enable_sessions(home.path()).unwrap();
    let handle = agent.sessions().unwrap();
    let id = handle.id();
    agent
        .run_turn("Create a file, then review it", &mut |_| Ok(()))
        .unwrap();
    let original = agent.messages.lock().unwrap().clone();
    fs::write(directory.path().join("shared.txt"), "external edit").unwrap();
    assert_eq!(agent.context.input_tokens, Some(73));
    agent.replace_backend(Box::new(beta.client())).unwrap();
    assert_eq!(agent.context.input_tokens, None);
    assert_eq!(agent.context.calibration, None);
    assert_eq!(*agent.messages.lock().unwrap(), original);
    assert_eq!(agent.sessions().unwrap().id(), id);
    agent
        .run_turn("Review the same file", &mut |_| Ok(()))
        .unwrap();
    let wire = beta.requests.lock().unwrap()[0].encode();
    assert!(wire.contains("turns"));
    assert!(wire.contains("Create a file, then review it"));
    assert!(wire.contains("write-once"));
    assert!(!wire.contains("alpha-private-state"));
    assert!(!wire.contains("jecode_origin"));
    assert!(!wire.contains("\"function\":"));
    agent.replace_backend(Box::new(alpha.client())).unwrap();
    agent
        .run_turn("Continue the review", &mut |_| Ok(()))
        .unwrap();
    let requests = alpha.requests.lock().unwrap();
    let returned = requests.last().unwrap().encode();
    assert!(returned.contains("dialog"));
    assert!(returned.contains("alpha-private-state"));
    assert!(!returned.contains("beta-private-state"));
    assert!(returned.contains("Review the same file"));
    drop(requests);
    assert_eq!(
        fs::read_to_string(directory.path().join("shared.txt")).unwrap(),
        "external edit"
    );
    assert_eq!(
        agent
            .messages
            .lock()
            .unwrap()
            .iter()
            .filter(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
            .count(),
        1
    );
    let saved = handle.store().fixture_load(&id).unwrap();
    assert_eq!(saved.backend, "alpha");
    assert_eq!(saved.messages, *agent.messages.lock().unwrap());

    // Compaction uses the same abstract request path with no tool catalog and
    // keeps the archived source messages intact across a backend change.
    agent.replace_backend(Box::new(beta.client())).unwrap();
    let archived = agent.messages.lock().unwrap().clone();
    let records = archived
        .iter()
        .enumerate()
        .skip(1)
        .map(|(index, message)| (index, conversation::portable(message).encode()))
        .collect::<Vec<_>>();
    let (summary, _) = agent
        .summarize(
            &records,
            "Review the file",
            Limits {
                context: 24000,
                output: None,
            },
            4096,
            &mut |_| Ok(()),
        )
        .unwrap();
    assert_eq!(summary, "Task and original file preserved");
    assert_eq!(*agent.messages.lock().unwrap(), archived);
    let requests = beta.requests.lock().unwrap();
    assert_eq!(
        requests.last().unwrap().get("operations"),
        Some(&Value::Null)
    );
    assert!(!requests.last().unwrap().encode().contains("private-state"));
    drop(requests);
    let saved_id = agent.sessions().unwrap().id();
    drop(handle);
    drop(agent);
    let mut resumed = Agent::new(alpha.client(), Tools::new(directory.path()).unwrap());
    resumed.register_backend(beta.services());
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&saved_id).unwrap();
    assert_eq!(resumed.backend_id(), "beta");
    assert_eq!(*resumed.messages.lock().unwrap(), archived);
    assert_eq!(resumed.context.input_tokens, None);
    assert_eq!(
        fs::read_to_string(directory.path().join("shared.txt")).unwrap(),
        "external edit"
    );
}

#[test]
fn a_failed_preparation_or_save_preserves_the_active_and_persisted_selection() {
    let directory = Directory::new();
    let home = Directory::new();
    let alpha = Account::new("alpha", vec![Reply::text("Ready")]);
    let beta = Account::new("beta", vec![]);
    let mut agent = Agent::new(alpha.client(), Tools::new(directory.path()).unwrap());
    agent.enable_sessions(home.path()).unwrap();
    agent
        .run_turn("Start the conversation", &mut |_| Ok(()))
        .unwrap();
    agent.context.observe(
        Some(&Usage {
            input_tokens: Some(99),
            ..Default::default()
        }),
        1,
    );
    let mut unavailable = beta.client();
    unavailable.fail_prepare = true;
    assert!(
        agent
            .replace_backend(Box::new(unavailable))
            .unwrap_err()
            .contains("unavailable")
    );
    assert_eq!(agent.backend_id(), "alpha");
    assert_eq!(agent.context.input_tokens, Some(99));
    let handle = agent.sessions().unwrap();
    let before = agent.archive().document();
    let path = handle
        .store()
        .bucket()
        .join(format!("{}.jsonl", handle.id()));
    let permissions = fs::metadata(&path).unwrap().permissions();
    let mut readonly = permissions.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&path, readonly).unwrap();
    let result = agent.replace_backend(Box::new(beta.client()));
    let model_result = agent.set_model("other-model".into());
    fs::set_permissions(&path, permissions).unwrap();
    assert!(result.is_err());
    assert!(model_result.is_err());
    assert_eq!(agent.backend_id(), "alpha");
    assert_eq!(agent.model(), "shared-model-name");
    assert_eq!(agent.context.input_tokens, Some(99));
    assert_eq!(
        agent.events.lock().unwrap().len(),
        before.get("events").unwrap().as_array().unwrap().len()
    );
    assert_eq!(handle.snapshot().backend, "alpha");
    assert_eq!(
        handle.snapshot().model.as_deref(),
        Some("shared-model-name")
    );
    agent.save_session().unwrap();
    assert_eq!(
        handle.store().fixture_load(&handle.id()).unwrap().backend,
        "alpha"
    );
    assert_eq!(alpha.requests.lock().unwrap().len(), 1);
    assert!(beta.requests.lock().unwrap().is_empty());
}

#[test]
fn legacy_native_fields_keep_their_original_model_after_switch_save_and_resume() {
    let directory = Directory::new();
    let home = Directory::new();
    let account = Account::new("openrouter", vec![]);
    let mut document = crate::sessions::Document::new(
        directory.path().into(),
        "shared-model-name".into(),
        Effort::Default,
    )
    .unwrap();
    document.messages = vec![
        Value::object([
            ("role", Value::string("system")),
            ("content", Value::string("System")),
        ]),
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Original task")),
        ]),
        Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string("Original answer")),
            ("native_state", Value::string("legacy-private-state")),
        ]),
    ];
    let original = document.messages.clone();
    let mut legacy = document.value();
    if let Value::Object(fields) = &mut legacy {
        fields.remove("backend");
        fields.remove("legacy_origin");
    }
    let document = crate::sessions::Document::parse(&legacy).unwrap();
    assert_eq!(
        document.legacy_origin,
        Some(conversation::Origin::new("openrouter", "shared-model-name"))
    );
    let store = crate::sessions::Store::new(home.path().into(), directory.path()).unwrap();
    let handle =
        crate::sessions::Handle::new(store.clone(), document, crate::redact::Redactor::empty());
    handle.flush().unwrap();
    let id = handle.id();
    let path = store.bucket().join(format!("{id}.jsonl"));
    let first_bytes = fs::read(&path).unwrap();
    drop(handle);
    let mut agent = Agent::new(account.client(), Tools::new(directory.path()).unwrap());
    agent.enable_sessions(home.path()).unwrap();
    agent.resume(&id).unwrap();
    agent.set_model("other-model".into()).unwrap();
    let current = agent.identity();
    let request = agent
        .transport_messages()
        .iter()
        .map(|message| conversation::project(message, &current))
        .collect::<Vec<_>>();
    assert!(
        !Value::Array(request)
            .encode()
            .contains("legacy-private-state")
    );
    agent.save_session().unwrap();
    drop(agent);
    let mut resumed = Agent::new(account.client(), Tools::new(directory.path()).unwrap());
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert_eq!(resumed.model(), "other-model");
    resumed.set_model("shared-model-name".into()).unwrap();
    let current = resumed.identity();
    let request = resumed
        .transport_messages()
        .iter()
        .map(|message| conversation::project(message, &current))
        .collect::<Vec<_>>();
    assert!(
        Value::Array(request)
            .encode()
            .contains("legacy-private-state")
    );
    assert_eq!(*resumed.messages.lock().unwrap(), original);
    assert_eq!(store.fixture_load(&id).unwrap().messages, original);
    assert!(fs::read(&path).unwrap().starts_with(&first_bytes));
    assert!(account.requests.lock().unwrap().is_empty());
}
