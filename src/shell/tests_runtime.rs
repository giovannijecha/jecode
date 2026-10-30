use super::tests::shell;
use super::*;
use std::io::Write;
use std::time::{Duration, Instant};

fn read(directory: &crate::test_support::Directory, name: &str) -> String {
    fs::read_to_string(directory.path().join(name)).unwrap()
}

fn helper() -> String {
    format!("'{}'", executable().unwrap().display())
}

#[test]
fn dev_null_discards_and_supplies_nothing_on_every_platform() {
    let (directory, mut shell) = shell();
    let script = "cargo --version >/dev/null 2>/dev/null && echo ok > ok; cat </dev/null > empty; echo hidden 2>/dev/null >/dev/null";
    assert_eq!(shell.run(script), 0);
    assert_eq!(read(&directory, "ok"), "ok\n");
    assert_eq!(read(&directory, "empty"), "");
    assert!(!directory.path().join("dev").exists());
}

#[test]
fn recursive_copy_refuses_a_destination_inside_the_source() {
    let (directory, mut shell) = shell();
    assert_eq!(
        shell.run("mkdir sub; touch sub/file; cp -r sub sub/inner"),
        1
    );
    assert!(!directory.path().join("sub/inner").exists());
    assert_eq!(shell.run("cp -r sub sub"), 1);
    assert_eq!(shell.run("cp -r sub copy"), 0);
    assert!(directory.path().join("copy/file").is_file());
}

#[test]
fn moving_across_volumes_copies_then_removes() {
    let (directory, mut shell) = shell();
    assert_eq!(
        shell.run("mkdir -p tree/nested; echo data > tree/nested/file"),
        0
    );
    builtins::copy_then_remove(
        &directory.path().join("tree"),
        &directory.path().join("moved"),
    )
    .unwrap();
    assert!(!directory.path().join("tree").exists());
    assert_eq!(read(&directory, "moved/nested/file"), "data\n");
}

#[test]
fn redirection_alone_creates_or_truncates_the_file() {
    let (directory, mut shell) = shell();
    assert_eq!(shell.run("echo data > kept; > kept; > created"), 0);
    assert_eq!(read(&directory, "kept"), "");
    assert_eq!(read(&directory, "created"), "");
}

#[test]
fn variable_names_follow_the_platform_environment() {
    let mut variables = Variables::default();
    variables.set("Path", "first".into());
    variables.set("PATH", "second".into());
    let entries: Vec<_> = variables.iter().collect();
    if cfg!(windows) {
        assert_eq!(entries, [("Path", "second")]);
        assert_eq!(variables.get("path"), Some("second"));
    } else {
        assert_eq!(entries.len(), 2);
        assert_eq!(variables.get("Path"), Some("first"));
    }
    let (directory, mut shell) = shell();
    assert_eq!(
        shell.run("export PATH=\"$PATH\"; test -n \"$PATH\" && echo set > found"),
        0
    );
    assert_eq!(read(&directory, "found"), "set\n");
}

#[test]
fn failing_substitution_still_substitutes_and_sets_status() {
    let (directory, mut shell) = shell();
    let script =
        "x=$(printf partial; exit 3); echo \"$?:$x\" > first; echo \"$(false)done\" > second";
    assert_eq!(shell.run(script), 0);
    assert_eq!(read(&directory, "first"), "3:partial\n");
    assert_eq!(read(&directory, "second"), "done\n");
    assert_eq!(shell.run("echo $(printf '%2000000s' y) > too_large"), 1);
    assert!(!directory.path().join("too_large").exists());
}

#[test]
fn assignments_prefix_pipeline_stages_background_jobs_and_builtins() {
    let (directory, mut shell) = shell();
    let script = format!(
        "X=piped {helper} -c 'echo $X' | cat > piped; X=bg {helper} -c 'echo $X' > bg & wait; X=scoped env > env; echo \"[$X]\" > after",
        helper = helper()
    );
    assert_eq!(shell.run(&script), 0);
    assert_eq!(read(&directory, "piped"), "piped\n");
    assert_eq!(read(&directory, "bg"), "bg\n");
    assert!(read(&directory, "env").contains("X=scoped\n"));
    assert_eq!(read(&directory, "after"), "[]\n");
}

#[test]
fn script_files_run_in_a_fresh_shell_with_arguments() {
    let (directory, mut shell) = shell();
    fs::write(
        directory.path().join("child.sh"),
        "echo \"$1-$2-$0\" > args\nX=inner\ncd ..\nexit 4\necho unreachable > after_exit\n",
    )
    .unwrap();
    assert_eq!(
        shell.run("X=outer; bash child.sh one two; echo \"$?:$X\" > after"),
        0
    );
    assert_eq!(read(&directory, "args"), "one-two-child.sh\n");
    assert_eq!(read(&directory, "after"), "4:outer\n");
    assert!(!directory.path().join("after_exit").exists());
}

/// A runaway producer must never hang the test runner if pipe closure regresses.
fn bounded_script(script: &str, directory: &crate::test_support::Directory) -> i32 {
    let mut child = command(script)
        .unwrap()
        .current_dir(directory.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status.code().unwrap_or(1);
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("shell pipeline did not stop after its reader closed: {script}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn closed_pipeline_stops_compound_producers_and_preserves_caller() {
    let directory = crate::test_support::Directory::new();
    fs::write(directory.path().join("data"), "payload\n").unwrap();
    for (index, producer) in [
        "while true; do echo data; done",
        "(while true; do echo data; done)",
        "while true; do (echo data); done",
        "while true; do cat data; done",
        "while true; do cat data | tr -d x; done",
    ]
    .iter()
    .enumerate()
    {
        let script = format!("{producer} | true; echo after > marker{index}");
        assert_eq!(bounded_script(&script, &directory), 0, "{producer}");
        assert_eq!(read(&directory, &format!("marker{index}")), "after\n");
    }
    let executable = helper();
    let script = format!(
        "while true; do echo data; done | {executable} -c 'true'; echo after > external_reader"
    );
    assert_eq!(bounded_script(&script, &directory), 0);
    assert_eq!(read(&directory, "external_reader"), "after\n");
    let script = format!(
        "{executable} -c 'while true; do echo data; done' | true; echo after > external_producer"
    );
    assert_eq!(bounded_script(&script, &directory), 0);
    assert_eq!(read(&directory, "external_producer"), "after\n");
}

/// Runs as an ordinary child process of the shell, outside its interpreter.
#[test]
fn external_writer_fixture() {
    if std::env::var_os("JECODE_SHELL_EXTERNAL_WRITER").is_none() {
        return;
    }
    fs::write("external_started", "yes").unwrap();
    let mut stdout = io::stdout();
    loop {
        match stdout.write_all(b"payload\n") {
            Ok(()) => {}
            Err(error) if streams::closed(&error) => {
                fs::write("external_closed", "yes").unwrap();
                std::process::exit(17);
            }
            Err(error) => panic!("external fixture output failed: {error}"),
        }
    }
}

#[test]
fn closed_pipeline_stops_a_loop_running_an_external_program() {
    let directory = crate::test_support::Directory::new();
    let executable = std::env::current_exe().unwrap();
    let mut direct = Command::new(&executable)
        .args([
            "--exact",
            "shell::tests_runtime::external_writer_fixture",
            "--nocapture",
        ])
        .env("JECODE_SHELL_EXTERNAL_WRITER", "1")
        .current_dir(directory.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !directory.path().join("external_started").exists() {
        assert!(Instant::now() < deadline, "external fixture did not start");
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(direct.stdout.take());
    let direct_result = direct.wait_with_output().unwrap();
    assert_eq!(
        direct_result.status.code(),
        Some(17),
        "{}",
        String::from_utf8_lossy(&direct_result.stderr)
    );
    fs::remove_file(directory.path().join("external_started")).unwrap();
    fs::remove_file(directory.path().join("external_closed")).unwrap();
    let fixture = format!("'{}'", executable.display());
    let script = format!(
        "while true; do JECODE_SHELL_EXTERNAL_WRITER=1 {fixture} --exact shell::tests_runtime::external_writer_fixture --nocapture 2> external_error; done | (until test -e external_started; do sleep 0.01; done; true); echo after > external_after"
    );
    assert_eq!(bounded_script(&script, &directory), 0);
    assert!(
        directory.path().join("external_closed").exists(),
        "{}",
        read(&directory, "external_error")
    );
    assert_eq!(read(&directory, "external_closed"), "yes");
    assert_eq!(read(&directory, "external_after"), "after\n");
}

#[test]
fn completed_reader_does_not_cancel_redirected_work() {
    let (directory, mut shell) = shell();
    assert_eq!(
        shell.run("for i in a b; do sleep 0.01; echo $i > marker; done | true"),
        0
    );
    assert_eq!(read(&directory, "marker"), "b\n");

    let fixture = format!("'{}'", std::env::current_exe().unwrap().display());
    let script = format!(
        "for i in a b; do {fixture} --exact shell::tests_runtime::external_writer_fixture > run_$i; done | true"
    );
    assert_eq!(bounded_script(&script, &directory), 0);
    assert!(directory.path().join("run_a").is_file());
    assert!(directory.path().join("run_b").is_file());
}

#[test]
fn substitution_inherits_the_active_compound_error_stream() {
    let (directory, mut shell) = shell();
    assert_eq!(
        shell.run("(echo \"$(echo diagnostic >&2; echo value)\") 2> err > out"),
        0
    );
    assert_eq!(read(&directory, "out"), "value\n");
    assert_eq!(read(&directory, "err"), "diagnostic\n");
    assert_eq!(
        shell.run("echo upstream | (echo \"$(echo nested >&2; cat)\") 2> piped_err > piped_out"),
        0
    );
    assert_eq!(read(&directory, "piped_out"), "upstream\n");
    assert_eq!(read(&directory, "piped_err"), "nested\n");
}
