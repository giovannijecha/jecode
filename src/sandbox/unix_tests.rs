//! Real Workspace confinement: Landlock on Linux and Seatbelt on macOS. Every
//! command goes through the tool path a model uses, so nothing here is simulated.

use super::*;
use crate::cancel::Cancellation;
use crate::redact::Redactor;
use crate::test_support::{Directory, sandbox as probe};
use crate::tools::{Begun, Tools};

/// Kernels without Landlock refuse Workspace commands; a skipped test says so.
pub(super) fn available() -> bool {
    let root = Directory::new();
    let tools = fixture(root.path());
    let result = exec(&tools, "/bin/sh", &["-c", "exit 0"]);
    let refused = result
        .get("error")
        .and_then(Value::as_str)
        .is_some_and(|error| error.contains("cannot be confined on this system"));
    if refused {
        #[cfg(target_os = "macos")]
        panic!("macOS must enforce Workspace confinement: {result:?}");
        // CI sets this: libtest hides the message of a passing test, so a kernel
        // without Landlock would otherwise leave these checks green and empty.
        #[cfg(target_os = "linux")]
        {
            assert!(
                std::env::var_os("JECODE_REQUIRE_CONFINEMENT").is_none(),
                "This job requires Workspace confinement: {result:?}"
            );
            eprintln!("SKIPPED: this kernel cannot confine commands: {result:?}");
        }
    } else {
        assert_exit(&result, 0);
    }
    !refused
}

pub(super) fn fixture(root: &Path) -> Tools {
    let mut tools = Tools::new(root).unwrap();
    tools.configure_output(root.join("logs"), Redactor::empty());
    tools
}

pub(super) fn run(tools: &Tools, name: &str, arguments: Value) -> Value {
    tools
        .execute_begun(
            Begun::default().with_permission(Level::Workspace, None),
            name,
            &arguments.encode(),
            &Cancellation::default(),
        )
        .result
}

pub(super) fn exec(tools: &Tools, program: &str, args: &[&str]) -> Value {
    run(
        tools,
        "exec",
        Value::object([
            ("executable", Value::string(program)),
            (
                "args",
                Value::Array(args.iter().map(|arg| Value::string(*arg)).collect()),
            ),
            ("timeout_seconds", Value::number(30)),
        ]),
    )
}

pub(super) fn text<'a>(value: &'a Value, name: &str) -> &'a str {
    value.get(name).and_then(Value::as_str).unwrap_or("")
}

#[track_caller]
pub(super) fn assert_exit(result: &Value, code: u32) {
    assert_eq!(
        result.get("exit_code"),
        Some(&Value::number(code)),
        "{result:?}"
    );
}

#[test]
fn child_and_grandchild_enforce_the_file_policy() {
    if !available() {
        return;
    }
    let root = Directory::new();
    let outside = Directory::new();
    let allowed = root.path().join("allowed.txt");
    let private = outside.path().join("private.txt");
    let escaped = outside.path().join("escaped.txt");
    std::fs::write(&allowed, "allowed").unwrap();
    std::fs::write(&private, "private").unwrap();
    let probe_file = probe::executable(root.path());
    let tools = fixture(root.path());
    let result = run(
        &tools,
        "exec",
        Value::object([
            ("executable", Value::string(probe_file.to_string_lossy())),
            (
                "env",
                Value::object([
                    (probe::ROLE, Value::string("parent")),
                    (probe::ALLOWED, Value::string(allowed.to_string_lossy())),
                    (probe::PRIVATE, Value::string(private.to_string_lossy())),
                    (probe::ESCAPED, Value::string(escaped.to_string_lossy())),
                ]),
            ),
            ("timeout_seconds", Value::number(10)),
        ]),
    );
    assert_exit(&result, 0);
    for expected in [
        "INSIDE:allowed",
        "OUTSIDE_ERROR:PermissionDenied",
        "WRITE_ERROR:PermissionDenied",
        "CREATED:created",
    ] {
        assert!(text(&result, "stdout").contains(expected), "{result:?}");
    }
    assert_eq!(std::fs::read_to_string(&private).unwrap(), "private");
    assert!(!escaped.exists());
}

/// Runs itself again with a secret in the parent's environment, the only way to
/// prove the secret stays out without changing this process's environment.
#[test]
fn commands_do_not_inherit_the_parent_environment() {
    const SECRET: &str = "JECODE_SECRET_CANARY";
    if !available() {
        return;
    }
    if std::env::var_os(SECRET).is_none() {
        let name = "sandbox::unix_tests::commands_do_not_inherit_the_parent_environment";
        let status = Command::new(std::env::current_exe().unwrap())
            .args([name, "--exact", "--nocapture"])
            .env(SECRET, "must-stay-out")
            .env("GH_TOKEN", "must-stay-out")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let root = Directory::new();
    let probe_file = probe::executable(root.path());
    let tools = fixture(root.path());
    let result = run(
        &tools,
        "exec",
        Value::object([
            ("executable", Value::string(probe_file.to_string_lossy())),
            (
                "env",
                Value::object([
                    (probe::ROLE, Value::string("environment")),
                    ("JECODE_TEST_ENV", Value::string("sentinel")),
                    (
                        probe::PRIVATE,
                        Value::string(format!("/proc/{}/environ", std::process::id())),
                    ),
                ]),
            ),
            ("timeout_seconds", Value::number(10)),
        ]),
    );
    assert_exit(&result, 0);
    assert!(
        text(&result, "stdout").contains("WORKSPACE_NATIVE:sentinel:"),
        "{result:?}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_session_home_in_a_system_runtime_folder_refuses_command_launch() {
    if !available() {
        return;
    }
    let root = Directory::new();
    let mut tools = fixture(root.path());
    tools.configure_session_home(Path::new("/opt"));
    let result = exec(&tools, "/bin/sh", &["-c", "printf must-not-run"]);
    assert!(
        text(&result, "error").contains("configuration overlaps a system runtime folder"),
        "{result:?}"
    );
    assert!(!text(&result, "stdout").contains("must-not-run"));
}

#[test]
fn the_policy_marker_never_reaches_the_command() {
    if !available() {
        return;
    }
    let root = Directory::new();
    let tools = fixture(root.path());
    let result = exec(
        &tools,
        "/bin/sh",
        &[
            "-c",
            "printf '[%s]' \"${JECODE_INTERNAL_SANDBOX_POLICY-absent}\"",
        ],
    );
    assert_exit(&result, 0);
    assert_eq!(text(&result, "stdout"), "[absent]");
}

#[test]
fn bash_runs_the_owned_shell_inside_the_policy() {
    if !available() {
        return;
    }
    let root = Directory::new();
    let outside = Directory::new();
    let private = outside.path().join("private.txt");
    std::fs::write(&private, "private").unwrap();
    let tools = fixture(root.path());
    let inside = run(
        &tools,
        "bash",
        Value::object([(
            "command",
            Value::string(
                "mkdir -p nested; printf owned > nested/marker.txt; cat nested/marker.txt",
            ),
        )]),
    );
    assert_exit(&inside, 0);
    assert_eq!(text(&inside, "stdout"), "owned");

    let denied = run(
        &tools,
        "bash",
        Value::object([(
            "command",
            Value::string(format!("cat '{}'", private.display())),
        )]),
    );
    assert_exit(&denied, 1);
    assert!(!text(&denied, "stdout").contains("private"), "{denied:?}");
}

#[test]
fn system_programs_devices_and_renames_inside_the_folder_work() {
    if !available() {
        return;
    }
    let root = Directory::new();
    std::fs::create_dir_all(root.path().join("from")).unwrap();
    std::fs::create_dir_all(root.path().join("to")).unwrap();
    std::fs::write(root.path().join("from/file"), "moved").unwrap();
    let tools = fixture(root.path());
    let script = "ls /usr/bin > /dev/null && echo out > /dev/stdout && echo err > /dev/stderr \
        && mv from/file to/file && cat to/file && ln -s to/file link && cat link \
        && chmod 600 to/file && rm link";
    let result = exec(&tools, "/bin/sh", &["-c", script]);
    assert_exit(&result, 0);
    assert_eq!(text(&result, "stdout"), "out\nmovedmoved");
    assert_eq!(text(&result, "stderr"), "err\n");
}

#[test]
fn a_link_inside_the_folder_does_not_reach_outside_contents() {
    if !available() {
        return;
    }
    let root = Directory::new();
    let outside = Directory::new();
    let private = outside.path().join("private.txt");
    std::fs::write(&private, "private").unwrap();
    std::os::unix::fs::symlink(&private, root.path().join("link")).unwrap();
    let tools = fixture(root.path());
    let read = exec(&tools, "/bin/sh", &["-c", "cat link"]);
    assert_exit(&read, 1);
    assert!(!text(&read, "stdout").contains("private"), "{read:?}");
    let write = exec(&tools, "/bin/sh", &["-c", "printf changed > link"]);
    assert_ne!(write.get("exit_code"), Some(&Value::number(0)), "{write:?}");
    assert_eq!(std::fs::read_to_string(&private).unwrap(), "private");
}

#[test]
fn a_program_outside_the_grants_is_refused_with_an_explanation() {
    if !available() {
        return;
    }
    let root = Directory::new();
    let outside = Directory::new();
    let program = probe::executable(outside.path());
    let tools = fixture(root.path());
    let result = exec(&tools, program.to_str().unwrap(), &[]);
    assert_exit(&result, 126);
    assert!(
        text(&result, "stderr").contains("outside what Workspace confinement allows"),
        "{result:?}"
    );
}

#[test]
fn an_invalid_policy_stops_the_command_from_starting() {
    let root = Directory::new();
    let marker = root.path().join("started");
    // Its own group, with stdin held open: the guardian kills the guarded group
    // as soon as stdin closes, which is how an owner stops a command.
    let mut child = std::os::unix::process::CommandExt::process_group(
        Command::new(crate::native::helper("process_unix").unwrap())
            .args(["guard", "/dev/null", "/bin/sh", "-c"])
            .arg(format!("echo started > '{}'", marker.display()))
            .env(MARKER, "?not a policy ("),
        0,
    )
    .stdin(std::process::Stdio::piped())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::null())
    .spawn()
    .unwrap();
    let owner = child.stdin.take();
    let mut header = Vec::new();
    std::io::Read::read_to_end(&mut child.stdout.take().unwrap(), &mut header).unwrap();
    let status = child.wait().unwrap();
    drop(owner);
    assert_eq!(status.code(), Some(125));
    assert_eq!(header, b"NOBOX\n");
    assert!(!marker.exists());
}

#[test]
fn compilers_and_git_work_with_the_temporary_area() {
    if !available() {
        return;
    }
    let root = Directory::new();
    let home = Directory::new();
    let mut tools = fixture(root.path());
    tools.configure_temporary(
        crate::scratch::Area::new(home.path().join("tmp"), root.path().to_path_buf(), "1-1-0")
            .unwrap(),
    );
    let script = "printf 'int main(void){return 0;}' > probe.c && cc probe.c -o probe && ./probe \
        && git init -q . && git add probe.c && git status --short && printf ok > \"$TMPDIR/note\" \
        && cat \"$TMPDIR/note\" && case \"$XDG_CACHE_HOME\" in \"$TMPDIR\"*) printf ' redirected';; esac";
    let result = exec(&tools, "/bin/sh", &["-c", script]);
    assert_exit(&result, 0);
    assert!(text(&result, "stdout").contains("A  probe.c"), "{result:?}");
    assert!(
        text(&result, "stdout").contains("ok redirected"),
        "{result:?}"
    );
}

/// Runs whatever this host has; a runtime that is installed must work.
#[test]
fn installed_runtimes_work_inside_the_policy() {
    if !available() {
        return;
    }
    let root = Directory::new();
    let home = Directory::new();
    let mut tools = fixture(root.path());
    tools.configure_temporary(
        crate::scratch::Area::new(home.path().join("tmp"), root.path().to_path_buf(), "1-1-0")
            .unwrap(),
    );
    for (name, script) in [
        (
            "cargo",
            // Its own `[workspace]`: this fixture sits inside Jecode's repository.
            "mkdir demo && cd demo && mkdir src && printf '[package]\\nname=\"demo\"\\nversion=\"0.1.0\"\\nedition=\"2021\"\\n[workspace]\\n' > Cargo.toml \
             && printf 'fn main(){println!(\"built\");}' > src/main.rs && cargo build --quiet --offline && ./target/debug/demo",
        ),
        (
            "node",
            "node -e \"require('fs').writeFileSync('n.txt','ok');console.log(require('fs').readFileSync('n.txt','utf8'))\"",
        ),
        ("npm", "npm --version"),
        (
            "python3",
            "python3 -c \"import tempfile,os;f=tempfile.NamedTemporaryFile();print(os.path.basename(f.name)!='')\"",
        ),
    ] {
        let script =
            format!("if command -v {name} > /dev/null; then {script}; else echo absent; fi");
        let result = exec(&tools, "/bin/sh", &["-c", &script]);
        assert_exit(&result, 0);
        eprintln!("{name}: {}", text(&result, "stdout").trim());
    }
}

#[test]
fn a_terminal_is_refused_rather_than_left_unconfined() {
    if !available() {
        return;
    }
    let root = Directory::new();
    let tools = fixture(root.path());
    let result = run(
        &tools,
        "exec",
        Value::object([
            ("executable", Value::string("/bin/sh")),
            ("tty", Value::Bool(true)),
        ]),
    );
    assert!(
        text(&result, "error").contains("interactive terminals"),
        "{result:?}"
    );
}

#[test]
fn a_workspace_command_can_reopen_its_anonymous_standard_input() {
    if !available() {
        return;
    }
    let root = Directory::new();
    let tools = fixture(root.path());
    let result = run(
        &tools,
        "exec",
        Value::object([
            ("executable", Value::string("/bin/cat")),
            ("args", Value::Array(vec![Value::string("/dev/stdin")])),
            ("stdin", Value::string("canary\n")),
            ("timeout_seconds", Value::number(2)),
        ]),
    );
    assert_exit(&result, 0);
    assert_eq!(text(&result, "stdout"), "canary\n");
}

#[test]
fn unix_programs_keep_their_identity_and_get_redirected_caches() {
    let project = Directory::new();
    let scratch = project.path().join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    let policy = Policy::for_command(Level::Workspace, project.path(), Some(&scratch), None)
        .unwrap()
        .unwrap();
    let mut command = Command::new("example");
    command.env("XDG_DATA_HOME", "explicit");
    confined_environment(
        &mut command,
        &policy,
        [
            (OsString::from("HOME"), OsString::from("/home/me")),
            (OsString::from("LC_ALL"), OsString::from("C.UTF-8")),
            (
                OsString::from("SSH_AUTH_SOCK"),
                OsString::from("/run/agent"),
            ),
            (
                OsString::from("AWS_SECRET_ACCESS_KEY"),
                OsString::from("secret"),
            ),
        ],
    );
    let value = |name: &str| {
        command
            .get_envs()
            .find(|(key, _)| *key == name)
            .and_then(|(_, value)| value)
            .map(|value| value.to_string_lossy().into_owned())
    };
    assert_eq!(value("HOME").as_deref(), Some("/home/me"));
    assert_eq!(value("LC_ALL").as_deref(), Some("C.UTF-8"));
    assert_eq!(value("SSH_AUTH_SOCK"), None);
    assert_eq!(value("AWS_SECRET_ACCESS_KEY"), None);
    assert_eq!(value("XDG_DATA_HOME").as_deref(), Some("explicit"));
    let temporary = std::fs::canonicalize(&scratch).unwrap();
    assert_eq!(
        value("XDG_CACHE_HOME").as_deref(),
        temporary.join("cache").to_str()
    );
}
