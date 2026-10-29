use super::*;
use crate::checkpoints::session::Session;
use crate::output::Store;
use crate::redact::Redactor;
use crate::test_support::Directory;
use crate::tools::{Tools, processes::Registry};
use std::io::Read;
use std::path::Path;
use std::sync::OnceLock;

fn fixture() -> &'static Path {
    static FIXTURE: OnceLock<(Directory, std::path::PathBuf)> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            let directory = Directory::new();
            let source = directory.path().join("exec_fixture.rs");
            std::fs::write(
                &source,
                r#"
use std::io::Read;
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    if args.first().map(String::as_str) == Some("--write") {
        std::fs::write("notes.txt", args.get(1).unwrap()).unwrap();
    }
    println!("args={args:?}");
    println!("stdin={input:?}");
    println!("cwd={}", std::env::current_dir().unwrap().display());
    println!("probe={:?}", std::env::var("JECODE_EXEC_PROBE").ok());
    println!("removed={:?}", std::env::var("JECODE_EXEC_REMOVED").ok());
    let provider = match std::env::var("OPENROUTER_API_KEY") {
        Err(_) => "unset",
        Ok(value) if value == "fixture-only" => "synthetic",
        Ok(_) => "other",
    };
    println!("provider={provider}");
    eprintln!("fixture stderr");
}
"#,
            )
            .unwrap();
            let executable = directory
                .path()
                .join(format!("exec_fixture{}", std::env::consts::EXE_SUFFIX));
            let mut compiler =
                Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()));
            compiler
                .arg(&source)
                .args(["--edition=2024", "-o"])
                .arg(&executable);
            if let Some(linker) = std::env::var_os("CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER")
            {
                let mut option = std::ffi::OsString::from("linker=");
                option.push(linker);
                compiler.arg("-C").arg(option);
            }
            let status = compiler
                .status()
                .expect("rustc must compile the original test fixture");
            assert!(status.success());
            (directory, executable)
        })
        .1
        .as_path()
}

fn run(root: &Path, arguments: &Value) -> Result<Value, String> {
    let outputs = Store::new(root.join("logs"), Redactor::empty());
    let processes = Registry::default();
    execute(
        shell::Context {
            bash: Path::new("unused"),
            root,
            outputs: &outputs,
            temporary: None,
            processes: &processes,
            sandbox: None,
        },
        arguments,
        &Cancellation::default(),
    )
}

#[test]
fn user_exec_env_cannot_replace_the_trusted_sandbox_policy() {
    let directory = Directory::new();
    let arguments = Value::object([
        ("executable", Value::string("example")),
        (
            "env",
            Value::object([(crate::sandbox::MARKER, Value::string("forged user policy"))]),
        ),
    ]);
    let (mut command, _, _) = command(directory.path(), None, &arguments).unwrap();
    let policy = crate::sandbox::Policy::for_command(
        crate::permissions::Level::Workspace,
        directory.path(),
        None,
        None,
    )
    .unwrap()
    .unwrap();
    let result = crate::sandbox::apply(&mut command, Some(&policy));
    if cfg!(windows) {
        result.unwrap();
        let marker = command
            .get_envs()
            .find(|(key, _)| *key == crate::sandbox::MARKER)
            .unwrap()
            .1
            .unwrap();
        assert!(marker.to_string_lossy().contains("\"network\":true"));
        assert_ne!(marker.to_string_lossy(), "forged user policy");
    } else {
        assert!(result.is_err());
    }
    crate::sandbox::apply(&mut command, None).unwrap();
    assert!(
        command
            .get_envs()
            .all(|(key, value)| key != crate::sandbox::MARKER || value.is_none())
    );
}

#[test]
fn command_label_matches_the_event_target_and_exact_arguments() {
    let directory = Directory::new();
    for (arguments, expected) in [
        (
            Value::object([("executable", Value::string("tool"))]),
            "tool",
        ),
        (
            Value::object([
                ("executable", Value::string("tool")),
                (
                    "args",
                    Value::Array(vec![
                        Value::string("a b"),
                        Value::string("\"quoted\""),
                        Value::string(""),
                    ]),
                ),
            ]),
            "tool [\"a b\",\"\\\"quoted\\\"\",\"\"]",
        ),
    ] {
        let (built, _, label) = command(directory.path(), None, &arguments).unwrap();
        assert_eq!(label, expected);
        assert_eq!(label, crate::events::target("exec", &arguments));
        let expected_args: Vec<_> = arguments
            .get("args")
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .map(|arg| arg.as_str().unwrap())
            .collect();
        assert_eq!(
            built
                .get_args()
                .map(|arg| arg.to_str().unwrap())
                .collect::<Vec<_>>(),
            expected_args
        );
    }
}

#[test]
fn nul_in_an_argument_is_rejected_without_launching_the_target() {
    let root = Directory::new();
    let result = run(
        root.path(),
        &Value::object([
            ("executable", Value::string(fixture().to_string_lossy())),
            ("args", Value::Array(vec![Value::string("before\0after")])),
        ]),
    );
    assert!(
        result.is_err(),
        "NUL argument was silently accepted: {result:?}"
    );
}

#[test]
fn passes_exact_arguments_stdin_cwd_and_environment_without_a_shell() {
    let directory = Directory::new();
    let child = directory.path().join("child");
    std::fs::create_dir(&child).unwrap();
    let executable = fixture().to_string_lossy().into_owned();
    let args = vec!["", "a b", "\"quote\"", "üñîçødé", "$(echo nope); & | *"];
    let arguments = Value::object([
        ("executable", Value::string(executable)),
        (
            "args",
            Value::Array(args.iter().map(|arg| Value::string(*arg)).collect()),
        ),
        ("cwd", Value::string("child")),
        ("stdin", Value::string("input\nü")),
        (
            "env",
            Value::object([
                ("JECODE_EXEC_PROBE", Value::string("kept")),
                ("JECODE_EXEC_REMOVED", Value::Null),
                ("OPENROUTER_API_KEY", Value::string("fixture-only")),
            ]),
        ),
    ]);
    let result = run(directory.path(), &arguments).unwrap();
    let stdout = result.get("stdout").and_then(Value::as_str).unwrap();
    assert!(stdout.contains(&format!("args={args:?}")), "{stdout}");
    assert!(stdout.contains("stdin=\"input\\nü\""), "{stdout}");
    let actual_cwd = stdout
        .lines()
        .find_map(|line| line.strip_prefix("cwd="))
        .unwrap();
    assert_eq!(
        Path::new(actual_cwd).canonicalize().unwrap(),
        child.canonicalize().unwrap()
    );
    assert!(stdout.contains("probe=Some(\"kept\")"), "{stdout}");
    assert!(stdout.contains("removed=None"), "{stdout}");
    assert!(stdout.contains("provider=synthetic"), "{stdout}");
    assert_eq!(
        result.get("stderr").and_then(Value::as_str),
        Some("fixture stderr\n")
    );
    assert_eq!(result.get("exit_code").and_then(Value::as_usize), Some(0));
}

#[test]
fn resolves_relative_executable_against_requested_cwd() {
    let directory = Directory::new();
    let child = directory.path().join("child");
    std::fs::create_dir(&child).unwrap();
    let executable = child.join(format!("exec_fixture{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(fixture(), &executable).unwrap();
    let result = run(
        directory.path(),
        &Value::object([
            (
                "executable",
                Value::string(format!(
                    ".{}{}",
                    std::path::MAIN_SEPARATOR,
                    executable.file_name().unwrap().to_string_lossy()
                )),
            ),
            ("cwd", Value::string("child")),
        ]),
    )
    .unwrap();
    assert_eq!(result.get("exit_code").and_then(Value::as_usize), Some(0));
    assert!(
        result
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap()
            .contains("provider=unset")
    );
}

#[test]
fn rejects_invalid_shapes_and_strips_only_inherited_credentials() {
    let directory = Directory::new();
    let root = directory.path();
    for (arguments, error) in [
        (
            Value::object([("executable", Value::string(""))]),
            "executable must not be empty",
        ),
        (
            Value::object([
                ("executable", Value::string("fixture")),
                ("args", Value::Array(vec![Value::number(1)])),
            ]),
            "args must contain only strings",
        ),
        (
            Value::object([
                ("executable", Value::string("fixture")),
                ("cwd", Value::string("missing")),
            ]),
            "Invalid cwd",
        ),
        (
            Value::object([
                ("executable", Value::string("fixture")),
                ("stdin", Value::number(1)),
            ]),
            "stdin must be a string",
        ),
    ] {
        assert!(command(root, None, &arguments).unwrap_err().contains(error));
    }
    let (built, _, _) = command(
        root,
        None,
        &Value::object([("executable", Value::string("fixture"))]),
    )
    .unwrap();
    assert!(
        built
            .get_envs()
            .any(|(name, value)| name == "OPENROUTER_API_KEY" && value.is_none())
    );
}

#[test]
fn session_temporary_environment_is_consistent_on_every_platform() {
    let directory = Directory::new();
    let (built, _, _) = command(
        directory.path(),
        Some(directory.path()),
        &Value::object([("executable", Value::string("fixture"))]),
    )
    .unwrap();
    let env: Vec<_> = built.get_envs().collect();
    for key in ["JECODE_TMP", "TMPDIR", "TEMP", "TMP"] {
        assert!(
            env.iter()
                .any(|(name, value)| *name == key && value.is_some())
        );
    }
}

#[test]
fn persistent_exec_receives_stdin_until_eof_and_preserves_saved_output() {
    let directory = Directory::new();
    let outputs = Store::new(directory.path().join("logs"), Redactor::empty());
    let processes = Registry::default();
    let input = "line one\nline two";
    let result = execute(
        shell::Context {
            bash: Path::new("unused"),
            root: directory.path(),
            outputs: &outputs,
            temporary: None,
            processes: &processes,
            sandbox: None,
        },
        &Value::object([
            ("executable", Value::string(fixture().to_string_lossy())),
            ("stdin", Value::string(input)),
            ("wait_ms", Value::number(0)),
            ("timeout_seconds", Value::number(15)),
        ]),
        &Cancellation::default(),
    )
    .unwrap();
    let id = result.get("process_id").and_then(Value::as_str).unwrap();
    let completed = processes
        .execute(
            &Value::object([
                ("action", Value::string("poll")),
                ("process_id", Value::string(id)),
                ("wait_ms", Value::number(15_000)),
            ]),
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(
        completed.get("status").and_then(Value::as_str),
        Some("exited"),
        "{completed:?}"
    );
    assert_eq!(
        completed.get("exit_code").and_then(Value::as_usize),
        Some(0)
    );
    let stdout = format!(
        "{}{}",
        result.get("stdout").and_then(Value::as_str).unwrap_or(""),
        completed
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or("")
    );
    assert!(stdout.contains(&format!("stdin={input:?}")), "{stdout}");
    let stderr = format!(
        "{}{}",
        result.get("stderr").and_then(Value::as_str).unwrap_or(""),
        completed
            .get("stderr")
            .and_then(Value::as_str)
            .unwrap_or("")
    );
    assert!(stderr.contains("fixture stderr"), "{stderr}");
}

#[test]
fn each_exec_protect_receipt_captures_the_prelaunch_file() {
    let root = Directory::new();
    let home = Directory::new();
    let path = root.path().join("notes.txt");
    std::fs::write(&path, "first").unwrap();
    let mut tools = Tools::new(root.path()).unwrap();
    tools.configure_output(home.path().join("logs"), Redactor::empty());
    let session = Session::offline(home.path(), tools.root());
    session.begin("exec-protect-fixture");
    tools.configure_checkpoints(session);
    for (before, after) in [("first", "second"), ("second", "third")] {
        let result = tools.execute(
            "exec",
            &Value::object([
                ("executable", Value::string(fixture().to_string_lossy())),
                (
                    "args",
                    Value::Array(vec![Value::string("--write"), Value::string(after)]),
                ),
                ("protect", Value::Array(vec![Value::string("notes.txt")])),
            ])
            .encode(),
        );
        assert_eq!(
            result.get("exit_code").and_then(Value::as_usize),
            Some(0),
            "{result:?}"
        );
        let checkpoint = result.get("checkpoint").unwrap();
        assert_eq!(
            checkpoint.get("status").and_then(Value::as_str),
            Some("saved")
        );
        let reference = checkpoint.get("ref").and_then(Value::as_str).unwrap();
        let store = tools.checkpoints().unwrap().store().unwrap();
        let loaded = store.load(reference, &Cancellation::default()).unwrap();
        let mut saved = String::new();
        loaded
            .reader(loaded.snapshot.entries.get("notes.txt").unwrap())
            .unwrap()
            .read_to_string(&mut saved)
            .unwrap();
        assert_eq!(saved, before);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), after);
    }
}
