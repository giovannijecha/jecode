//! The real executable: exit codes, stream separation, help and read-only
//! commands, each with an isolated Jecode home and no provider.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Home(PathBuf);

impl Home {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join("home")).unwrap();
        fs::create_dir_all(path.join("project")).unwrap();
        Self(path)
    }

    fn home(&self) -> PathBuf {
        self.0.join("home")
    }

    fn project(&self) -> PathBuf {
        self.0.join("project")
    }

    fn run(&self, arguments: &[&str], input: &str) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_jecode"))
            .args(arguments)
            .current_dir(self.project())
            .env("JECODE_HOME", self.home())
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("OPENROUTER_MODEL")
            .env_remove("JECODE_UPDATE_REGISTRY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn configure(&self) {
        fs::write(
            self.home().join("config.json"),
            r#"{"selection":{"backend":"openrouter","model":"fixture/model","effort":"default"},"backends":{"openrouter":{"api_key":"isolated-cli-fixture-key","model":"fixture/model"}}}"#,
        )
        .unwrap();
    }

    fn entries(&self) -> usize {
        fs::read_dir(self.home()).unwrap().count()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn help_goes_to_stdout_and_never_runs_the_command() {
    let home = Home::new();
    for arguments in [
        vec!["--help"],
        vec!["help"],
        vec!["resume", "--help"],
        vec!["setup", "--help"],
        vec!["sessions", "delete", "1-2", "--help"],
        vec!["uninstall", "-h"],
        vec!["help", "update"],
    ] {
        let output = home.run(&arguments, "");
        assert_eq!(output.status.code(), Some(0), "{arguments:?}");
        assert!(text(&output.stdout).contains("Usage:"), "{arguments:?}");
        assert!(
            output.stderr.is_empty(),
            "{arguments:?}: {}",
            text(&output.stderr)
        );
    }
    assert_eq!(home.entries(), 0, "help must not create configuration");
    let version = home.run(&["--version"], "");
    assert!(text(&version.stdout).starts_with("jecode "));
}

#[test]
fn invalid_usage_exits_2_with_stderr_only_before_any_side_effect() {
    let home = Home::new();
    for arguments in [
        vec!["--plain", "setup"],
        vec!["--frobnicate"],
        vec!["--effort", "extreme", "task"],
        vec!["--provider", "chatgpt", "task"],
        vec!["--routing", "bad endpoint", "task"],
        vec!["explain", "--attach", "a.png"],
        vec!["update", "now"],
        vec!["sessions", "show"],
        vec!["sessions", "show", "../escape"],
        vec!["resume", "not-an-id"],
        vec!["resume", "123-456", "--last", "prompt"],
        vec!["completions", "tcsh"],
        vec!["-C", "missing directory", "sessions"],
        vec!["resume", "--last", "--plain", "--json"],
    ] {
        let output = home.run(&arguments, "");
        assert_eq!(
            output.status.code(),
            Some(2),
            "{arguments:?}: {}",
            text(&output.stderr)
        );
        assert!(output.stdout.is_empty(), "{arguments:?}");
        assert!(
            text(&output.stderr).starts_with("jecode: "),
            "{arguments:?}"
        );
    }
    assert_eq!(home.entries(), 0);
}

#[test]
fn read_only_commands_work_without_model_credentials() {
    let home = Home::new();
    for arguments in [
        vec!["settings"],
        vec!["settings", "path"],
        vec!["providers"],
        vec!["providers", "accounts", "openai"],
        vec!["providers", "accounts", "anthropic"],
        vec!["sessions"],
        vec!["skills"],
        vec!["mcp"],
        vec!["lsp"],
        vec!["doctor"],
        vec!["completions", "powershell"],
    ] {
        let output = home.run(&arguments, "");
        assert_eq!(
            output.status.code(),
            Some(0),
            "{arguments:?}: {}",
            text(&output.stderr)
        );
        assert!(!output.stdout.is_empty(), "{arguments:?}");
    }
    let json = home.run(&["settings", "show", "--json"], "");
    let value = text(&json.stdout);
    assert!(value.trim_start().starts_with('{') && value.contains("\"configured\": false"));
    assert_eq!(home.entries(), 0, "inspection must not write the home");
    // A task needs configuration and fails without waiting or writing.
    let task = home.run(&["Explain this project"], "");
    assert_eq!(task.status.code(), Some(1));
    assert!(text(&task.stderr).contains("jecode setup"));
    assert_eq!(home.entries(), 0);
}

#[test]
fn explicit_provider_from_an_empty_home_uses_its_own_route() {
    let home = Home::new();
    let output = home.run(
        &["--provider", "openai", "--model", "fixture/model", "task"],
        "",
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output.stderr).contains("OpenAI has no active ChatGPT account"),
        "{}",
        text(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert_eq!(home.entries(), 0);

    let incompatible = home.run(
        &[
            "--provider",
            "openai",
            "--model",
            "fixture/model",
            "--routing",
            "auto",
            "task",
        ],
        "",
    );
    assert_eq!(incompatible.status.code(), Some(1));
    assert!(text(&incompatible.stderr).contains("only available with openrouter"));
    assert_eq!(home.entries(), 0);

    let model_only = home.run(&["--model", "fixture/model", "task"], "");
    assert_eq!(model_only.status.code(), Some(1));
    assert!(text(&model_only.stderr).contains("Jecode is not configured"));
    assert_eq!(home.entries(), 0);
}

#[test]
fn saved_accounts_can_be_inspected_without_a_default_model() {
    let home = Home::new();
    let accounts = home.run(&["providers", "accounts", "openai", "--json"], "");
    assert_eq!(
        accounts.status.code(),
        Some(0),
        "{}",
        text(&accounts.stderr)
    );
    let value = text(&accounts.stdout);
    assert!(value.contains("\"accounts\": []"), "{value}");
    assert_eq!(home.entries(), 0);
    let wrong_provider = home.run(&["providers", "accounts", "openrouter"], "");
    assert_eq!(wrong_provider.status.code(), Some(2));
    let wrong_logout = home.run(
        &[
            "providers",
            "logout",
            "openrouter",
            "--account",
            "id",
            "--yes",
        ],
        "",
    );
    assert_eq!(wrong_logout.status.code(), Some(2));
    assert_eq!(home.entries(), 0);
    let incompatible = home.run(
        &[
            "models",
            "endpoints",
            "fixture/model",
            "--provider",
            "openai-api",
            "--json",
        ],
        "",
    );
    assert_eq!(incompatible.status.code(), Some(2));
    assert!(incompatible.stdout.is_empty());
    assert_eq!(home.entries(), 0);
}

#[test]
fn json_output_and_saved_settings_never_include_credentials() {
    let home = Home::new();
    home.configure();
    for arguments in [
        vec!["settings", "--json"],
        vec!["providers", "--json"],
        vec!["doctor", "--json"],
        vec!["settings", "get", "model"],
    ] {
        let output = home.run(&arguments, "");
        let all = format!("{}{}", text(&output.stdout), text(&output.stderr));
        assert!(
            !all.contains("isolated-cli-fixture-key"),
            "{arguments:?}: {all}"
        );
    }
    assert_eq!(
        text(&home.run(&["settings", "get", "model"], "").stdout).trim(),
        "fixture/model"
    );
    let before = fs::read(home.home().join("config.json")).unwrap();
    let invalid = home.run(&["settings", "set", "theme", "neon"], "");
    assert_eq!(invalid.status.code(), Some(2));
    assert_eq!(fs::read(home.home().join("config.json")).unwrap(), before);
    let changed = home.run(&["settings", "set", "theme", "light"], "");
    assert_eq!(changed.status.code(), Some(0), "{}", text(&changed.stderr));
    assert_eq!(
        text(&home.run(&["settings", "get", "theme"], "").stdout).trim(),
        "light"
    );
    // Removing a key without a terminal needs --yes rather than waiting.
    let refused = home.run(&["providers", "logout", "openrouter"], "");
    assert_eq!(refused.status.code(), Some(2));
    assert!(text(&refused.stderr).contains("--yes"));
}

#[test]
fn unmanaged_builds_are_reported_rather_than_updated_or_removed() {
    let home = Home::new();
    for arguments in [vec!["update"], vec!["uninstall", "--yes"]] {
        let output = home.run(&arguments, "");
        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        assert!(
            text(&output.stderr)
                .contains("not installed by npm, Volta or the standalone installer"),
            "{arguments:?}: {}",
            text(&output.stderr)
        );
    }
    assert!(Path::new(env!("CARGO_BIN_EXE_jecode")).exists());
}
