use crate::json::Value;
use crate::redact::Redactor;
use crate::test_support::Directory;
use crate::tools::Tools;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(10);

#[path = "interactive_tests/validation.rs"]
mod validation;

fn fixture() -> &'static Path {
    static FIXTURE: OnceLock<(Directory, PathBuf)> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            let directory = Directory::new();
            let source = directory.path().join("terminal_fixture.rs");
            std::fs::write(
                &source,
                r#"
use std::io::{self, BufRead, IsTerminal, Write};
use std::time::Duration;

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.first().map(String::as_str) == Some("heartbeat_child") {
        loop {
            let mut file = std::fs::OpenOptions::new().create(true).append(true)
                .open(&arguments[1]).unwrap();
            file.write_all(b"x").unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    if arguments.first().map(String::as_str) == Some("heartbeat") {
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("heartbeat_child").arg(&arguments[1]).spawn().unwrap();
        println!("HEARTBEAT:{}", child.id());
        io::stdout().flush().unwrap();
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line).unwrap();
        return;
    }
    println!("TTY:{}:{}:{}", io::stdin().is_terminal(),
        io::stdout().is_terminal(), io::stderr().is_terminal());
    println!("ARGS:{arguments:?}");
    println!("CWD:{}", std::env::current_dir().unwrap().display());
    println!("ENV:{}", std::env::var("JECODE_TTY_PROBE").unwrap_or_default());
    eprintln!("STDERR:merged");
    println!("READY");
    io::stdout().flush().unwrap();
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line).unwrap();
    println!("INPUT:{:?}", line.trim_end_matches(['\r', '\n']));
    std::process::exit(17);
}
"#,
            )
            .unwrap();
            let executable = directory
                .path()
                .join(format!("terminal_fixture{}", std::env::consts::EXE_SUFFIX));
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
            assert!(compiler.status().expect("rustc must run").success());
            (directory, executable)
        })
        .1
        .as_path()
}

fn tools(root: &Directory) -> Tools {
    let mut tools = Tools::new(root.path()).unwrap();
    tools.configure_output(root.path().join("logs"), Redactor::empty());
    tools
}

fn text<'a>(value: &'a Value, name: &str) -> &'a str {
    value
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{name} missing from {value:?}"))
}

fn execute(tools: &Tools, executable: &Path, args: &[&str], extra: &[(&str, Value)]) -> Value {
    let mut arguments = Value::object([
        ("executable", Value::string(executable.to_string_lossy())),
        (
            "args",
            Value::Array(args.iter().map(|arg| Value::string(*arg)).collect()),
        ),
        ("tty", Value::Bool(true)),
        ("wait_ms", Value::number(0)),
    ]);
    if let Value::Object(fields) = &mut arguments {
        for (name, value) in extra {
            fields.insert((*name).into(), value.clone());
        }
    }
    let result = tools.execute("exec", &arguments.encode());
    assert!(
        result.get("error").is_none(),
        "PTY launch failed: {result:?}"
    );
    result
}

fn action(tools: &Tools, id: &str, name: &str, extra: &[(&str, Value)]) -> Value {
    let mut arguments = Value::object([
        ("action", Value::string(name)),
        ("process_id", Value::string(id)),
        ("wait_ms", Value::number(200)),
    ]);
    if let Value::Object(fields) = &mut arguments {
        for (name, value) in extra {
            fields.insert((*name).into(), value.clone());
        }
    }
    let result = tools.execute("process", &arguments.encode());
    assert!(
        result.get("error").is_none(),
        "process {name} failed: {result:?}"
    );
    result
}

fn collect_until(tools: &Tools, id: &str, output: &mut String, marker: &str) -> Value {
    let started = Instant::now();
    loop {
        let result = action(tools, id, "poll", &[]);
        output.push_str(text(&result, "stdout"));
        assert!(
            started.elapsed() < WAIT,
            "PTY did not show {marker:?}: {output}; last={result:?}"
        );
        if output.contains(marker) {
            return result;
        }
        assert_eq!(
            text(&result, "status"),
            "running",
            "PTY ended before {marker:?}: {output}; last={result:?}"
        );
    }
}

fn finish(tools: &Tools, id: &str, output: &mut String) -> Value {
    let started = Instant::now();
    loop {
        let result = action(tools, id, "poll", &[]);
        output.push_str(text(&result, "stdout"));
        assert!(started.elapsed() < WAIT, "PTY did not finish: {result:?}");
        if text(&result, "status") != "running" {
            return result;
        }
    }
}

#[test]
fn terminal_exec_preserves_argv_cwd_env_and_streams_across_process_actions() {
    let root = Directory::new();
    let child = root.path().join("child");
    std::fs::create_dir(&child).unwrap();
    let tools = tools(&root);
    let started = execute(
        &tools,
        fixture(),
        &["a b", "$literal; &"],
        &[
            ("cwd", Value::string("child")),
            (
                "env",
                Value::object([("JECODE_TTY_PROBE", Value::string("preserved"))]),
            ),
            ("cols", Value::number(91)),
            ("rows", Value::number(32)),
        ],
    );
    let id = text(&started, "process_id").to_owned();
    assert_eq!(text(&started, "status"), "running");
    assert_eq!(started.get("tty"), Some(&Value::Bool(true)));
    assert_eq!(started.get("cols").and_then(Value::as_usize), Some(91));
    let listed = tools.execute("process", r#"{"action":"list"}"#);
    assert!(
        listed
            .get("processes")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .any(|entry| text(entry, "process_id") == id
                && entry.get("tty") == Some(&Value::Bool(true)))
    );
    let mut output = text(&started, "stdout").to_owned();
    let ready = collect_until(&tools, &id, &mut output, "READY");
    assert_eq!(text(&ready, "status"), "running");
    assert!(output.contains("TTY:true:true:true"), "{output}");
    assert!(
        output.contains("ARGS:[\"a b\", \"$literal; &\"]"),
        "{output}"
    );
    assert!(output.contains("ENV:preserved"), "{output}");
    assert!(output.contains("STDERR:merged"), "{output}");
    let cwd = output
        .lines()
        .find_map(|line| line.split_once("CWD:").map(|(_, path)| path))
        .unwrap()
        .trim_end_matches('\r');
    assert_eq!(
        Path::new(cwd).canonicalize().unwrap(),
        child.canonicalize().unwrap()
    );
    assert_eq!(text(&started, "stderr"), "");
    let sent = action(&tools, &id, "write", &[("text", Value::string("hello"))]);
    output.push_str(text(&sent, "stdout"));
    assert_eq!(text(&sent, "status"), "running");
    let entered = action(&tools, &id, "write", &[("key", Value::string("Enter"))]);
    output.push_str(text(&entered, "stdout"));
    let ended = finish(&tools, &id, &mut output);
    assert_eq!(text(&ended, "status"), "exited");
    assert_eq!(ended.get("exit_code").and_then(Value::as_usize), Some(17));
    assert!(output.contains("INPUT:\"hello\""), "{output}");
    assert_eq!(text(&ended, "stderr"), "");
    let again = action(&tools, &id, "poll", &[]);
    assert_eq!(text(&again, "status"), "exited");
    assert_eq!(text(&again, "stdout"), "");
    let invalid = tools.execute(
        "exec",
        &Value::object([
            ("executable", Value::string(fixture().to_string_lossy())),
            ("tty", Value::Bool(true)),
            ("stdin", Value::string("use process write")),
        ])
        .encode(),
    );
    assert!(text(&invalid, "error").contains("Use process write"));
    assert!(invalid.get("process_id").is_none());
}

#[test]
fn stopping_a_terminal_stops_its_descendant_and_conversation_end_does_too() {
    for stop_conversation in [false, true] {
        let root = Directory::new();
        let tools = tools(&root);
        let marker = root.path().join("heartbeat");
        let path = marker.to_string_lossy().into_owned();
        let started = execute(&tools, fixture(), &["heartbeat", &path], &[]);
        let id = text(&started, "process_id").to_owned();
        let beginning = Instant::now();
        while std::fs::metadata(&marker).map_or(0, |entry| entry.len()) < 2 {
            assert!(
                beginning.elapsed() < WAIT,
                "descendant did not write: {started:?}"
            );
            thread::sleep(Duration::from_millis(20));
        }
        if stop_conversation {
            tools.stop_processes();
        } else {
            let stopped = action(&tools, &id, "stop", &[]);
            assert_eq!(text(&stopped, "status"), "stopped");
        }
        let length = std::fs::metadata(&marker).unwrap().len();
        thread::sleep(Duration::from_millis(300));
        assert_eq!(
            std::fs::metadata(&marker).unwrap().len(),
            length,
            "terminal descendant survived its owner"
        );
    }
}

#[cfg(unix)]
#[test]
fn interactive_bash_job_groups_end_on_stop_and_conversation_change() {
    for stop_conversation in [false, true] {
        let root = Directory::new();
        let tools = tools(&root);
        let heartbeat = root.path().join("job-heartbeat");
        let shell_pid = root.path().join("shell-pid");
        let job_pid = root.path().join("job-pid");
        let script = r#"printf '%s' "$$" > "$JECODE_TTY_SHELL"; while :; do printf x >> "$JECODE_TTY_HEARTBEAT"; sleep 0.02; done & printf '%s' "$!" > "$JECODE_TTY_JOB"; wait"#;
        let started = execute(
            &tools,
            Path::new("/bin/bash"),
            &["--noprofile", "--norc", "-i", "-c", script],
            &[(
                "env",
                Value::object([
                    (
                        "JECODE_TTY_HEARTBEAT",
                        Value::string(heartbeat.to_string_lossy()),
                    ),
                    (
                        "JECODE_TTY_SHELL",
                        Value::string(shell_pid.to_string_lossy()),
                    ),
                    ("JECODE_TTY_JOB", Value::string(job_pid.to_string_lossy())),
                ]),
            )],
        );
        let id = text(&started, "process_id").to_owned();
        let beginning = Instant::now();
        while std::fs::metadata(&heartbeat).map_or(0, |entry| entry.len()) < 2 {
            assert!(
                beginning.elapsed() < WAIT,
                "background job did not start: {started:?}"
            );
            thread::sleep(Duration::from_millis(20));
        }
        let shell = std::fs::read_to_string(&shell_pid).unwrap();
        let job = std::fs::read_to_string(&job_pid).unwrap();
        let groups = Command::new("ps")
            .args(["-o", "pgid=", "-p"])
            .arg(format!("{},{}", shell.trim(), job.trim()))
            .output()
            .unwrap();
        assert!(groups.status.success());
        let groups = String::from_utf8(groups.stdout).unwrap();
        let groups: Vec<_> = groups.split_whitespace().collect();
        assert_eq!(
            groups.len(),
            2,
            "could not inspect Bash job control: {groups:?}"
        );
        assert_ne!(
            groups[0], groups[1],
            "interactive Bash did not create a separate job group"
        );
        if stop_conversation {
            tools.stop_processes();
        } else {
            let stopped = action(&tools, &id, "stop", &[]);
            assert_eq!(text(&stopped, "status"), "stopped");
        }
        let length = std::fs::metadata(&heartbeat).unwrap().len();
        thread::sleep(Duration::from_millis(300));
        assert_eq!(
            std::fs::metadata(&heartbeat).unwrap().len(),
            length,
            "interactive Bash job survived terminal cleanup"
        );
    }
}

#[cfg(unix)]
fn resize_command() -> (&'static Path, Vec<&'static str>, &'static str, &'static str) {
    (
        Path::new("/bin/sh"),
        vec![
            "-c",
            r#"printf 'READY\n'; IFS= read -r line; printf 'SIZE1:%s\n' "$(stty size)"; IFS= read -r line; printf 'SIZE2:%s\n' "$(stty size)"; printf 'WAITING\n'; trap 'printf "INTERRUPTED\n"; exit 130' INT; while :; do sleep 1; done"#,
        ],
        "SIZE1:37 101",
        "SIZE2:41 107",
    )
}

#[cfg(windows)]
fn resize_command() -> (&'static Path, Vec<&'static str>, &'static str, &'static str) {
    (
        Path::new("powershell.exe"),
        vec![
            "-NoLogo",
            "-NoProfile",
            "-Command",
            "[Console]::WriteLine('READY'); [Console]::ReadLine() | Out-Null; [Console]::WriteLine(('SIZE1:{0}x{1}' -f [Console]::WindowWidth, [Console]::WindowHeight)); [Console]::ReadLine() | Out-Null; [Console]::WriteLine(('SIZE2:{0}x{1}' -f [Console]::WindowWidth, [Console]::WindowHeight)); [Console]::WriteLine('WAITING'); while ($true) { Start-Sleep -Milliseconds 100 }",
        ],
        "SIZE1:101x37",
        "SIZE2:107x41",
    )
}

#[test]
fn native_terminal_resizes_and_interrupts_through_model_and_local_controls() {
    let root = Directory::new();
    let tools = tools(&root);
    let (program, args, first_size, second_size) = resize_command();
    let started = execute(&tools, program, &args, &[]);
    let id = text(&started, "process_id").to_owned();
    let mut output = text(&started, "stdout").to_owned();
    let ready = collect_until(&tools, &id, &mut output, "READY");
    assert_eq!(text(&ready, "status"), "running");

    let resized = action(
        &tools,
        &id,
        "resize",
        &[("cols", Value::number(101)), ("rows", Value::number(37))],
    );
    output.push_str(text(&resized, "stdout"));
    assert_eq!(resized.get("cols").and_then(Value::as_usize), Some(101));
    let entered = action(&tools, &id, "write", &[("key", Value::string("Enter"))]);
    output.push_str(text(&entered, "stdout"));
    let _ = collect_until(&tools, &id, &mut output, first_size);
    assert!(
        output.contains(first_size),
        "first native size was wrong: {output}"
    );

    let observer = tools.observe_processes(Redactor::empty());
    observer.resize_direct(&id, (107, 41)).unwrap();
    assert_eq!(observer.terminal_size(&id), Some((107, 41)));
    observer.write_direct(&id, b"\r").unwrap();
    let _ = collect_until(&tools, &id, &mut output, second_size);
    assert!(
        output.contains(second_size),
        "second native size was wrong: {output}"
    );
    let waiting = collect_until(&tools, &id, &mut output, "WAITING");
    assert_eq!(text(&waiting, "status"), "running");
    let interrupted = action(&tools, &id, "interrupt", &[]);
    output.push_str(text(&interrupted, "stdout"));
    let ended = finish(&tools, &id, &mut output);
    assert_eq!(text(&ended, "status"), "exited");
    #[cfg(unix)]
    assert_eq!(ended.get("exit_code").and_then(Value::as_usize), Some(130));
}

#[cfg(unix)]
fn secret_command() -> (&'static Path, Vec<&'static str>) {
    (
        Path::new("/bin/sh"),
        vec![
            "-c",
            r#"stty -echo; printf 'READY\n'; IFS= read -r secret; stty echo; printf 'LEN:%s\n' "${#secret}""#,
        ],
    )
}

#[cfg(windows)]
fn secret_command() -> (&'static Path, Vec<&'static str>) {
    (
        Path::new("powershell.exe"),
        vec![
            "-NoLogo",
            "-NoProfile",
            "-Command",
            "[Console]::WriteLine('READY'); $secret = Read-Host -AsSecureString; [Console]::WriteLine('LEN:{0}', $secret.Length)",
        ],
    )
}

#[test]
fn local_sensitive_input_reaches_terminal_without_appearing_in_tool_output() {
    let root = Directory::new();
    let tools = tools(&root);
    let (program, args) = secret_command();
    let started = execute(&tools, program, &args, &[]);
    let id = text(&started, "process_id").to_owned();
    let mut output = text(&started, "stdout").to_owned();
    let ready = collect_until(&tools, &id, &mut output, "READY");
    assert_eq!(text(&ready, "status"), "running");
    let secret = "fixture-secret-8472";
    let observer = tools.observe_processes(Redactor::empty());
    let mut bytes = secret.as_bytes().to_vec();
    bytes.push(b'\r');
    observer.write_direct(&id, &bytes).unwrap();
    let ended = finish(&tools, &id, &mut output);
    assert_eq!(text(&ended, "status"), "exited");
    assert!(
        output.contains(&format!("LEN:{}", secret.len())),
        "{output}"
    );
    assert!(
        !output.contains(secret),
        "sensitive input was echoed: {output}"
    );
    assert!(!text(&ended, "stderr").contains(secret));
}
