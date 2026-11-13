use super::*;
use crate::config::Settings;
use crate::test_support::Directory;
use std::fs;

fn command(action: &'static str, operands: &[&str]) -> Manage {
    Manage {
        family: "providers",
        action,
        operands: operands.iter().map(|value| (*value).into()).collect(),
        values: vec![],
    }
}

fn shown(report: &Report) -> String {
    format!("{}{}", report.text, report.value.pretty())
}

fn saved_claude(home: &std::path::Path, id: &str, label: &str, token: &str) {
    let digest = crate::auth::openai::sha256(id.as_bytes(), &Cancellation::default()).unwrap();
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    let file = home
        .join("accounts/anthropic")
        .join(format!("account-{hex}.json"));
    let record = Value::object([
        ("format", Value::string("jecode-anthropic-account-v1")),
        ("id", Value::string(id)),
        ("label", Value::string(label)),
        ("access_token", Value::string(token)),
        ("refresh_token", Value::string("second-refresh-token")),
        ("expires_at", Value::number(u64::MAX)),
        ("scopes", Value::string("user:profile user:inference")),
    ]);
    fs::write(file, format!("{}\n", record.encode())).unwrap();
}

#[test]
fn account_listing_reads_empty_home_without_creating_files() {
    let directory = Directory::new();
    let context = Context::at(directory.path());
    let report = run(&context, &command("accounts", &["openai"])).unwrap();
    assert!(report.text.contains("No saved"));
    assert_eq!(
        report
            .value
            .get("accounts")
            .and_then(Value::as_array)
            .unwrap()
            .len(),
        0
    );
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn saved_accounts_can_be_listed_selected_and_signed_out_by_id() {
    let directory = Directory::new();
    let context = Context::at(directory.path());
    let settings = Settings::new("isolated-api-key".into(), "fixture/model".into()).unwrap();
    context.store.save(&settings).unwrap();
    let configuration = fs::read(context.store.path()).unwrap();
    crate::auth::anthropic::Store::fixture_account(
        directory.path().to_path_buf(),
        "fixture-claude-access-token",
    )
    .unwrap();
    saved_claude(
        directory.path(),
        "second-account",
        "Second",
        "second-claude-access-token",
    );
    let store = AccountStore::new(directory.path().to_path_buf(), "anthropic");
    let list = run(&context, &command("accounts", &["anthropic"])).unwrap();
    let output = shown(&list);
    assert!(output.contains("fixture-account") && output.contains("second-account"));
    assert!(!output.contains("fixture-claude-access-token"));
    assert!(!output.contains("second-claude-access-token"));
    let accounts = list
        .value
        .get("accounts")
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(accounts.len(), 2);
    assert_eq!(accounts[0].get("active"), Some(&Value::Bool(true)));
    assert_eq!(accounts[1].get("active"), Some(&Value::Bool(false)));

    let use_report = run(&context, &command("use", &["anthropic", "second-account"])).unwrap();
    assert!(shown(&use_report).contains("Second"));
    assert_eq!(store.active().unwrap().unwrap().id(), "second-account");
    assert_eq!(fs::read(context.store.path()).unwrap(), configuration);

    let mut logout = command("logout", &["anthropic"]);
    logout.values = vec![("account", Some("fixture-account".into())), ("yes", None)];
    let report = run(&context, &logout).unwrap();
    assert_eq!(
        report.value.get("account").and_then(Value::as_str),
        Some("fixture-account")
    );
    assert_eq!(store.active().unwrap().unwrap().id(), "second-account");
    assert!(
        !store
            .session("fixture-account")
            .unwrap()
            .account()
            .unwrap()
            .enabled
    );
    assert!(!shown(&report).contains("fixture-claude-access-token"));
    assert_eq!(fs::read(context.store.path()).unwrap(), configuration);

    let mut active_logout = command("logout", &["anthropic"]);
    active_logout.values.push(("yes", None));
    run(&context, &active_logout).unwrap();
    assert!(store.active().unwrap().is_none());
}

#[test]
fn account_labels_mask_credentials_from_other_providers() {
    let directory = Directory::new();
    let context = Context::at(directory.path());
    let key = "isolated-openrouter-secret-in-label";
    context
        .store
        .save(&Settings::new(key.into(), "fixture/model".into()).unwrap())
        .unwrap();
    crate::auth::anthropic::Store::fixture_account(
        directory.path().to_path_buf(),
        "fixture-claude-access-token",
    )
    .unwrap();
    saved_claude(
        directory.path(),
        "second-account",
        key,
        "second-access-token",
    );
    for command in [
        command("accounts", &["anthropic"]),
        command("use", &["anthropic", "second-account"]),
    ] {
        let report = run(&context, &command).unwrap();
        assert!(!report.text.contains(key), "{}", report.text);
        assert!(
            !report.value.pretty().contains(key),
            "{}",
            report.value.pretty()
        );
        assert!(report.text.contains("[redacted]"));
    }
}

#[test]
fn account_actions_reject_api_key_providers_before_mutation() {
    let directory = Directory::new();
    let context = Context::at(directory.path());
    let settings = Settings::new("isolated-api-key".into(), "fixture/model".into()).unwrap();
    context.store.save(&settings).unwrap();
    let before = fs::read(context.store.path()).unwrap();
    for command in [
        command("accounts", &["openrouter"]),
        command("use", &["openrouter", "fixture-account"]),
        Manage {
            values: vec![("account", Some("fixture-account".into())), ("yes", None)],
            ..command("logout", &["openrouter"])
        },
    ] {
        assert!(matches!(run(&context, &command), Err(Failure::Usage(_))));
        assert_eq!(fs::read(context.store.path()).unwrap(), before);
    }
}
