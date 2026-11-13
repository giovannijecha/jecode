use super::*;
use crate::config::Settings;
use crate::test_support::Directory;
use std::fs;

const KEY: &str = "isolated-settings-fixture-key";

fn command(family: &'static str, action: &'static str, operands: &[&str]) -> Manage {
    Manage {
        family,
        action,
        operands: operands
            .iter()
            .map(|operand| (*operand).to_owned())
            .collect(),
        values: vec![("yes", None)],
    }
}

fn configured(directory: &Directory) -> Context {
    let context = Context::at(directory.path());
    let settings = Settings::new(KEY.into(), "fixture/saved".into()).unwrap();
    context.store.save(&settings).unwrap();
    context
}

fn printed(report: &Report) -> String {
    let mut text = Vec::new();
    report.print(false, &mut text).unwrap();
    let mut json = Vec::new();
    report.print(true, &mut json).unwrap();
    format!(
        "{}{}",
        String::from_utf8(text).unwrap(),
        String::from_utf8(json).unwrap()
    )
}

fn failure(result: Result<Report, Failure>) -> (bool, String) {
    match result {
        Err(Failure::Usage(message)) => (true, message),
        Err(Failure::Error(message)) => (false, message),
        Ok(report) => panic!("expected a failure, got {}", report.text),
    }
}

#[test]
fn report_prints_text_or_pretty_json_on_stdout_only() {
    let report = Report::new(Value::object([("key", Value::string("value"))]), "text");
    let mut text = Vec::new();
    report.print(false, &mut text).unwrap();
    assert_eq!(text, b"text\n");
    let mut json = Vec::new();
    report.print(true, &mut json).unwrap();
    let parsed = crate::json::parse(std::str::from_utf8(&json).unwrap()).unwrap();
    assert_eq!(parsed.get("key").and_then(Value::as_str), Some("value"));
    let mut nothing = Vec::new();
    Report::done().print(true, &mut nothing).unwrap();
    assert!(nothing.is_empty());
}

#[test]
fn read_only_inspection_needs_no_configuration_and_writes_nothing() {
    let directory = Directory::new();
    let context = Context::at(directory.path());
    let report = settings::run(&context, &command("settings", "show", &[])).unwrap();
    assert!(report.text.contains("not configured"), "{}", report.text);
    assert_eq!(report.value.get("configured"), Some(&Value::Bool(false)));
    for (family, action) in [
        ("providers", "list"),
        ("skills", "list"),
        ("mcp", "list"),
        ("lsp", "list"),
    ] {
        let manage = command(family, action, &[]);
        match family {
            "providers" => providers::run(&context, &manage),
            "skills" => skills::run(&context, &manage),
            "mcp" => mcp::run(&context, &manage),
            _ => lsp::run(&context, &manage),
        }
        .unwrap();
    }
    let project = Directory::new();
    sessions::run_in(
        &context,
        &command("sessions", "list", &[]),
        project.path(),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    let (usage, _) = failure(settings::run(
        &context,
        &command("settings", "set", &["theme", "light"]),
    ));
    assert!(!usage, "a missing configuration is not a usage error");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn settings_change_saved_defaults_through_the_shared_parsers() {
    let directory = Directory::new();
    let context = configured(&directory);
    for (key, value, shown) in [
        ("theme", "light", "light"),
        ("permissions", "read", "read"),
        ("auto-compact", "65%", "65%"),
        ("prompt-cache", "1h", "1h"),
        ("effort", "default", "default"),
        ("routing", "auto", "auto"),
    ] {
        let report = settings::run(&context, &command("settings", "set", &[key, value])).unwrap();
        assert!(report.text.contains(shown), "{key}: {}", report.text);
        let got = settings::run(&context, &command("settings", "get", &[key])).unwrap();
        assert_eq!(got.text, shown, "{key}");
    }
    let saved = context.store.load().unwrap().unwrap();
    assert_eq!(saved.theme, crate::config::Theme::Light);
    assert_eq!(saved.default_permissions, crate::permissions::Level::Read);
    assert_eq!(saved.auto_compact, 65);
    let before = fs::read(context.store.path()).unwrap();
    for (key, value) in [
        ("theme", "neon"),
        ("permissions", "admin"),
        ("auto-compact", "100"),
        ("prompt-cache", "2h"),
        ("effort", "extreme"),
        ("provider", "openai"),
        ("colour", "blue"),
    ] {
        let (usage, message) = failure(settings::run(
            &context,
            &command("settings", "set", &[key, value]),
        ));
        assert!(usage, "{key}={value}: {message}");
    }
    // Another provider's model needs that provider's credentials.
    let mut set = command("settings", "set", &["model", "gpt-fixture"]);
    set.values.push(("provider", Some("openai-api".into())));
    let (usage, message) = failure(settings::run(&context, &set));
    assert!(
        !usage && message.contains("providers login openai-api"),
        "{message}"
    );
    assert_eq!(fs::read(context.store.path()).unwrap(), before);
    let report = settings::run(
        &context,
        &command("settings", "set", &["model", "fixture/next"]),
    )
    .unwrap();
    assert!(report.text.contains("fixture/next"));
    let shown = printed(&settings::run(&context, &command("settings", "show", &[])).unwrap());
    assert!(
        shown.contains("fixture/next") && !shown.contains(KEY),
        "{shown}"
    );
}

#[test]
fn providers_keep_routes_distinct_and_never_print_credentials() {
    let directory = Directory::new();
    let context = configured(&directory);
    crate::auth::openai::Store::fixture_account(
        directory.path().to_path_buf(),
        "isolated-access-token",
    )
    .unwrap();
    let report = providers::run(&context, &command("providers", "list", &[])).unwrap();
    let shown = printed(&report);
    assert!(
        !shown.contains(KEY) && !shown.contains("isolated-access-token"),
        "{shown}"
    );
    let rows = report
        .value
        .get("providers")
        .and_then(Value::as_array)
        .unwrap();
    let row = |id: &str| {
        rows.iter()
            .find(|row| row.get("id").and_then(Value::as_str) == Some(id))
            .unwrap()
    };
    assert_eq!(
        row("openrouter").get("billing").and_then(Value::as_str),
        Some("API billing")
    );
    assert_eq!(row("openrouter").get("ready"), Some(&Value::Bool(true)));
    assert_eq!(
        row("openai").get("billing").and_then(Value::as_str),
        Some("ChatGPT plan")
    );
    assert_eq!(row("openai").get("ready"), Some(&Value::Bool(true)));
    assert_eq!(row("openai-api").get("ready"), Some(&Value::Bool(false)));
    // Removing a key leaves the rest of the file, including the model, as written.
    let report =
        providers::run(&context, &command("providers", "logout", &["openrouter"])).unwrap();
    assert!(!printed(&report).contains(KEY));
    let text = fs::read_to_string(context.store.path()).unwrap();
    assert!(
        !text.contains(KEY) && text.contains("fixture/saved"),
        "{text}"
    );
    let (_, message) = failure(providers::run(
        &context,
        &command("providers", "logout", &["openrouter"]),
    ));
    assert!(message.contains("no saved key"));
    let (usage, _) = failure(providers::run(
        &context,
        &command("providers", "login", &["chatgpt"]),
    ));
    assert!(usage);
}

#[test]
fn removing_a_key_keeps_a_near_limit_configuration_loadable() {
    let directory = Directory::new();
    let context = configured(&directory);
    let mut document =
        crate::json::parse(&fs::read_to_string(context.store.path()).unwrap()).unwrap();
    let Value::Object(root) = &mut document else {
        unreachable!()
    };
    root.insert("extension".into(), Value::string(""));
    let baseline = document.encode().len() + 1;
    let Value::Object(root) = &mut document else {
        unreachable!()
    };
    root.insert(
        "extension".into(),
        Value::string("x".repeat(64 * 1024 - baseline)),
    );
    let original = format!("{}\n", document.encode());
    assert_eq!(original.len(), 64 * 1024);
    fs::write(context.store.path(), original).unwrap();
    assert!(context.store.remove_credential("openrouter").unwrap());
    let saved = fs::read_to_string(context.store.path()).unwrap();
    assert!(saved.len() <= 64 * 1024);
    assert!(context.store.load().is_ok());
    assert!(!saved.contains(KEY));
}

fn session(directory: &Directory, project: &std::path::Path, text: &str) -> String {
    let fixture = crate::test_support::HttpFixture::new(vec![(
        200,
        crate::test_support::completion("Saved answer.", vec![]),
    )]);
    let mut agent = crate::agent::Agent::new(
        crate::backend::openrouter::OpenRouter::fixture(fixture.endpoint.clone()),
        crate::tools::Tools::new(project).unwrap(),
    );
    agent.enable_sessions(directory.path()).unwrap();
    agent
        .run_turn(
            crate::attachments::Prompt::plain(text),
            &mut |_event: crate::events::Event| Ok(()),
        )
        .unwrap();
    agent.save_session().unwrap();
    let id = agent.sessions().unwrap().id();
    drop(agent);
    assert_eq!(fixture.finish().len(), 1);
    id
}

#[test]
fn sessions_list_show_export_and_delete_without_opening_a_conversation() {
    let directory = Directory::new();
    let context = configured(&directory);
    let project = Directory::new();
    let id = session(&directory, project.path(), &format!("Explain {KEY} please"));
    let run = |manage: Manage| {
        let mut output = Vec::new();
        let report = sessions::run_in(&context, &manage, project.path(), &mut output);
        (report, output)
    };
    let (listed, _) = run(command("sessions", "list", &[]));
    let listed = printed(&listed.unwrap());
    assert!(listed.contains(&id));
    assert!(!listed.contains(KEY), "{listed}");
    let (shown, _) = run(command("sessions", "show", &[&id]));
    let shown = printed(&shown.unwrap());
    assert!(
        shown.contains("Saved answer.") && !shown.contains(KEY),
        "{shown}"
    );
    let target = project.path().join("export.json");
    let mut export = command("sessions", "export", &[&id]);
    export
        .values
        .push(("output", Some(target.to_string_lossy().into_owned())));
    run(Manage {
        values: export.values.clone(),
        ..command("sessions", "export", &[&id])
    })
    .0
    .unwrap();
    let exported = fs::read_to_string(&target).unwrap();
    assert!(exported.contains("jecode.conversation") && !exported.contains(KEY));
    let (again, _) = run(export);
    assert!(failure(again).1.contains("never replaced"));
    let mut stdout = command("sessions", "export", &[&id]);
    stdout.values.push(("output", Some("-".into())));
    let (report, output) = run(stdout);
    assert!(report.unwrap().text.is_empty());
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("jecode.conversation")
    );
    let store =
        crate::sessions::Store::new(directory.path().to_path_buf(), project.path()).unwrap();
    let kept = store.session_directory(&id).join(KEY);
    fs::write(&kept, "unowned fixture").unwrap();
    let (deleted, _) = run(command("sessions", "delete", &[&id]));
    let deleted = printed(&deleted.unwrap());
    assert!(deleted.contains("Deleted"));
    assert!(!deleted.contains(KEY), "{deleted}");
    assert!(kept.exists());
    let (listed, _) = run(command("sessions", "list", &[]));
    assert!(!listed.unwrap().text.contains(&id));
    let (missing, _) = run(command("sessions", "show", &[&id]));
    assert!(failure(missing).1.contains("not found"));
}

#[test]
fn sessions_list_does_not_rebuild_a_missing_summary_cache() {
    let directory = Directory::new();
    let context = configured(&directory);
    let project = Directory::new();
    let id = session(&directory, project.path(), "Saved prompt");
    let store =
        crate::sessions::Store::new(directory.path().to_path_buf(), project.path()).unwrap();
    let summary = store
        .session_directory(&id)
        .join(crate::sessions::layout::SUMMARY);
    fs::remove_file(&summary).unwrap();
    let report = sessions::run_in(
        &context,
        &command("sessions", "list", &[]),
        project.path(),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(report.text.contains(&id));
    assert!(!summary.exists());
}

#[test]
fn unsupported_saved_sessions_are_reported_and_left_unchanged() {
    let directory = Directory::new();
    let context = configured(&directory);
    let project = Directory::new();
    let id = session(&directory, project.path(), "keep");
    let store =
        crate::sessions::Store::new(directory.path().to_path_buf(), project.path()).unwrap();
    let journal = store
        .session_directory(&id)
        .join(crate::sessions::layout::JOURNAL);
    fs::write(
        &journal,
        "{\"format\":\"jecode.session\",\"format_version\":1}\n",
    )
    .unwrap();
    let before = fs::read(&journal).unwrap();
    let run = |action: &'static str| {
        sessions::run_in(
            &context,
            &command("sessions", action, &[&id]),
            project.path(),
            &mut Vec::new(),
        )
    };
    let listed = sessions::run_in(
        &context,
        &command("sessions", "list", &[]),
        project.path(),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(
        listed.text.contains(&format!("Session {id}")),
        "{}",
        listed.text
    );
    for action in ["show", "export", "delete"] {
        let (_, message) = failure(run(action));
        assert!(!message.is_empty(), "{action}");
        assert_eq!(
            fs::read(&journal).unwrap(),
            before,
            "{action} changed the file"
        );
    }
}

#[test]
fn mcp_and_lsp_management_share_stores_and_mask_credentials() {
    let directory = Directory::new();
    let context = Context::at(directory.path());
    let mut connection =
        crate::mcp::Connection::draft("Docs", "https://example.test/mcp?key=query-secret");
    connection.method = crate::mcp::AuthMethod::Token;
    connection.token = "isolated-mcp-token-value".into();
    let saved = crate::mcp::Store::new(directory.path().to_path_buf())
        .save(&connection)
        .unwrap();
    for action in ["list", "show"] {
        let operands: &[&str] = if action == "show" { &["docs"] } else { &[] };
        let shown = printed(&mcp::run(&context, &command("mcp", action, operands)).unwrap());
        assert!(
            !shown.contains("isolated-mcp-token-value") && !shown.contains("query-secret"),
            "{shown}"
        );
        assert!(shown.contains("Docs"));
    }
    mcp::run(&context, &command("mcp", "disable", &["Docs"])).unwrap();
    let store = crate::mcp::Store::new(directory.path().to_path_buf());
    assert!(!store.list().unwrap()[0].enabled);
    mcp::run(&context, &command("mcp", "enable", &[&saved.id])).unwrap();
    assert!(store.list().unwrap()[0].enabled);
    let (_, message) = failure(mcp::run(&context, &command("mcp", "show", &["missing"])));
    assert!(message.contains("No MCP connection"));
    mcp::run(&context, &command("mcp", "remove", &["docs"])).unwrap();
    assert!(store.list().unwrap().is_empty());

    let mut add = command("lsp", "add", &["rust-analyzer"]);
    add.values
        .push(("executable", Some("missing-rust-analyzer-fixture".into())));
    lsp::run(&context, &add).unwrap();
    let servers = crate::lsp::Store::new(directory.path().to_path_buf())
        .load()
        .unwrap();
    assert_eq!(servers.len(), 1);
    assert!(servers[0].enabled && servers[0].executable == "missing-rust-analyzer-fixture");
    let mut server = servers[0].clone();
    server
        .environment
        .insert("API_TOKEN".into(), "isolated-lsp-secret-value".into());
    crate::lsp::Store::new(directory.path().to_path_buf())
        .save(server.clone())
        .unwrap();
    let shown = printed(&lsp::run(&context, &command("lsp", "show", &["rust-analyzer"])).unwrap());
    assert!(
        !shown.contains("isolated-lsp-secret-value") && shown.contains("API_TOKEN"),
        "{shown}"
    );
    lsp::run(&context, &command("lsp", "disable", &[&server.id])).unwrap();
    assert!(
        !crate::lsp::Store::new(directory.path().to_path_buf())
            .load()
            .unwrap()[0]
            .enabled
    );
    let (usage, _) = failure(lsp::run(&context, &command("lsp", "add", &["haskell"])));
    assert!(usage);
    lsp::run(&context, &command("lsp", "remove", &["rust-analyzer"])).unwrap();
    assert!(
        crate::lsp::Store::new(directory.path().to_path_buf())
            .load()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn lsp_check_failure_does_not_persist_or_return_a_configured_secret() {
    let directory = Directory::new();
    let context = Context::at(directory.path());
    let mut profile = crate::lsp::Profile::draft_guided_rust();
    let secret = "isolated-lsp-executable-secret";
    profile.executable = secret.into();
    profile
        .environment
        .insert("API_TOKEN".into(), secret.into());
    let id = profile.id.clone();
    crate::lsp::Store::new(directory.path().to_path_buf())
        .save(profile)
        .unwrap();
    let (_, error) = failure(lsp::run(&context, &command("lsp", "check", &[&id])));
    assert!(!error.contains(secret), "{error}");
    let stored = fs::read_to_string(directory.path().join("lsp.json")).unwrap();
    assert_eq!(stored.matches(secret).count(), 2, "{stored}");
    let shown = printed(&lsp::run(&context, &command("lsp", "show", &[&id])).unwrap());
    assert!(!shown.contains(secret), "{shown}");
}

#[test]
fn skills_list_and_show_read_the_personal_library() {
    let directory = Directory::new();
    let context = Context::at(directory.path());
    let package = directory.path().join("skills/review");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("SKILL.md"),
        "---\nname: review\ndescription: Inspect a change\n---\n\nRead the diff first.\n",
    )
    .unwrap();
    let listed = skills::run(&context, &command("skills", "list", &[])).unwrap();
    assert!(
        listed.text.contains("review  Inspect a change"),
        "{}",
        listed.text
    );
    let shown = skills::run(&context, &command("skills", "show", &["review"])).unwrap();
    assert!(shown.text.contains("Read the diff first."));
    let (_, message) = failure(skills::run(
        &context,
        &command("skills", "show", &["absent"]),
    ));
    assert!(message.contains("not in the current catalog"));
}

#[test]
fn targets_resolve_by_exact_id_then_by_a_unique_name() {
    let entries = || {
        [("a1", "rust"), ("b2", "Rust"), ("c3", "go")]
            .map(|(id, name)| (id.to_owned(), name.to_owned(), id))
    };
    assert_eq!(resolve(entries(), "b2", "server", "lsp").unwrap(), "b2");
    assert_eq!(resolve(entries(), "GO", "server", "lsp").unwrap(), "c3");
    let Err(Failure::Error(several)) = resolve(entries(), "rust", "server", "lsp") else {
        panic!("a repeated name must not pick one entry");
    };
    assert!(several.contains("a1, b2"), "{several}");
    assert!(resolve(entries(), "none", "server", "lsp").is_err());
}
