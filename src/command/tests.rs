//! Native subprocess fixtures are this owned test binary, not model or user data.
use super::*;
use crate::workspace_fixture::Fixture;
use std::{
    io::Write,
    sync::atomic::{AtomicBool, Ordering},
};

pub(crate) fn script(mode: &str) -> String {
    let exe = std::env::current_exe().unwrap();
    let exe = exe.to_str().unwrap();
    #[cfg(windows)]
    let script = format!(
        "$env:JECODE_COMMAND_FIXTURE='{mode}'; & '{}' --exact command::tests::child_fixture --nocapture --test-threads=1",
        exe.replace('\'', "''")
    );
    #[cfg(not(windows))]
    let script = format!(
        "JECODE_COMMAND_FIXTURE='{mode}' '{}' --exact command::tests::child_fixture --nocapture --test-threads=1",
        exe.replace('\'', "'\\''")
    );
    script
}
#[test]
#[allow(clippy::zombie_processes)] // Deliberately leave a descendant for the executor's job/group cleanup.
fn child_fixture() {
    let Ok(mode) = std::env::var("JECODE_COMMAND_FIXTURE") else {
        return;
    };
    match mode.as_str() {
        "success" => {
            let mut input = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut input).unwrap();
            assert!(input.is_empty(), "child stdin must be closed");
            println!("cwd={}", std::env::current_dir().unwrap().display());
            for byte in "café 中文\n\u{1b}[31m! forged\u{202e}\n".as_bytes() {
                std::io::stdout().write_all(&[*byte]).unwrap();
                std::io::stdout().flush().unwrap();
            }
            eprintln!("separate error output");
        }
        "write" => {
            std::fs::write("command-result.txt", "one execution\n").unwrap();
            println!("written");
        }
        "failure" => {
            eprintln!("expected failure");
            std::process::exit(7);
        }
        "long" => {
            println!("{}\nlast output", "line of output\n".repeat(3500));
        }
        "flood" => {
            std::io::stdout()
                .write_all(&vec![b'x'; 2 * 1024 * 1024])
                .unwrap();
        }
        "tree" | "exit-tree" => {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child
                .args([
                    "--exact",
                    "command::tests::child_fixture",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("JECODE_COMMAND_FIXTURE", "stream")
                .stdin(std::process::Stdio::null());
            if mode == "exit-tree" {
                child
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
            }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                child.creation_flags(0x08000000);
            }
            let mut child = child.spawn().unwrap();
            println!("descendant={}", child.id());
            std::io::stdout().flush().unwrap();
            if mode == "tree" {
                let _ = child.wait();
            }
        }
        "stream" => {
            println!("stream-ready");
            std::io::stdout().flush().unwrap();
            thread::sleep(Duration::from_secs(30));
        }
        _ => panic!("unexpected fixture mode"),
    }
}
fn launch(
    mode: &str,
    seconds: u64,
    output: &mut dyn FnMut(Channel, &str) -> ControlFlow<()>,
) -> Outcome {
    let files = Fixture::new();
    let workspace = Workspace::open(&files.0).unwrap();
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(40),
    };
    let proposal = prepare(&workspace, &script(mode), ".", seconds, &budget).unwrap();
    run(proposal, &workspace, &budget, output).unwrap()
}
#[test]
fn real_command_preserves_streams_exit_codes_unicode_and_starting_directory() {
    let mut seen = String::new();
    let result = launch("success", 20, &mut |_, text| {
        seen.push_str(text);
        ControlFlow::Continue(())
    });
    assert!(result.success(), "{}\n{}", result.summary(), result.stderr);
    assert!(result.stdout.contains("cwd="));
    assert!(result.stdout.contains("workspace-tests"));
    assert!(result.stdout.contains("café 中文"), "{}", result.stdout);
    assert!(result.stderr.contains("separate error output"));
    assert!(!seen.contains('\u{1b}'));
    assert!(!seen.contains('\u{202e}'));
    assert!(seen.contains("\\u{1b}[31m! forged\\u{202e}"));
    let failed = launch("failure", 20, &mut |_, _| ControlFlow::Continue(()));
    assert!(!failed.success());
    assert_eq!(failed.exit.unwrap().code, Some(7), "{}", failed.summary());
    assert!(failed.stderr.contains("expected failure"));
}
#[test]
fn streaming_cancellation_and_normal_exit_clean_up_ordinary_descendants() {
    for mode in ["tree", "exit-tree"] {
        let mut stream = String::new();
        let started = Instant::now();
        let result = launch(mode, 20, &mut |_, text| {
            stream.push_str(text);
            if mode == "tree" && stream.contains("stream-ready") {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        assert!(!result.cleanup_failed);
        assert!(started.elapsed() < Duration::from_secs(15));
        assert_eq!(
            result.stop,
            if mode == "tree" {
                Stop::Cancelled
            } else {
                Stop::Exited
            }
        );
        let start = stream.find("descendant=").unwrap() + "descendant=".len();
        let pid: u32 = stream[start..]
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert_stopped(pid);
    }
}
pub(crate) fn assert_stopped(pid: u32) {
    #[cfg(windows)]
    {
        #[allow(unsafe_code)]
        fn active(pid: u32) -> bool {
            use std::ffi::c_void;
            unsafe extern "system" {
                fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
                fn WaitForSingleObject(handle: *mut c_void, ms: u32) -> u32;
                fn CloseHandle(handle: *mut c_void) -> i32;
            }
            // SAFETY: query-only handle to the fixture's reported PID, closed once.
            unsafe {
                let handle = OpenProcess(0x00100000, 0, pid);
                if handle.is_null() {
                    return false;
                }
                let running = WaitForSingleObject(handle, 0) == 258;
                CloseHandle(handle);
                running
            }
        }
        assert!(!active(pid), "descendant {pid} still active");
    }
    #[cfg(target_os = "linux")]
    {
        // Non-child descendants are reaped by their adopter, not by this worker.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
            if stat.is_empty()
                || stat
                    .split_once(") ")
                    .is_some_and(|(_, state)| state.starts_with(['Z', 'X']))
            {
                break;
            }
            assert!(Instant::now() < deadline, "descendant {pid} still running");
            thread::sleep(Duration::from_millis(10));
        }
    }
}
#[test]
fn timeout_and_output_limits_keep_bounded_tail_and_report_partial_results() {
    let result = launch("stream", 1, &mut |_, _| ControlFlow::Continue(()));
    assert_eq!(result.stop, Stop::Timeout);
    assert!(!result.cleanup_failed);
    let mut displayed = 0;
    let result = launch("long", 20, &mut |_, text| {
        displayed += text.len();
        ControlFlow::Continue(())
    });
    assert!(result.success());
    assert!(result.truncated);
    assert!(result.stdout.len() <= 6144);
    assert!(result.stdout.contains("last output"));
    assert_eq!(displayed, 32768);
    let flood = launch("flood", 20, &mut |_, _| ControlFlow::Continue(()));
    assert_eq!(flood.stop, Stop::OutputLimit);
    assert!(flood.truncated && !flood.cleanup_failed);
}
#[test]
fn proposals_are_inert_and_reject_stale_directories_hidden_commands_and_cancellation() {
    let files = Fixture::new();
    files.write("sub/keep", "unchanged");
    let workspace = Workspace::open(&files.0).unwrap();
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(20),
    };
    for script in ["", "echo \u{1b}[31m", "echo \u{202e}bad", "line\rreturn"] {
        assert!(prepare(&workspace, script, ".", 10, &budget).is_err());
    }
    for path in ["../outside", "sub/../", ".env", "absent"] {
        assert!(prepare(&workspace, "echo safe", path, 10, &budget).is_err());
    }
    let proposal = prepare(&workspace, &script("write"), ".", 10, &budget).unwrap();
    assert!(!files.0.join("command-result.txt").exists());
    cancelled.store(true, Ordering::Release);
    assert!(
        run(proposal, &workspace, &budget, &mut |_, _| {
            ControlFlow::Continue(())
        })
        .is_err()
    );
    assert!(!files.0.join("command-result.txt").exists());
    cancelled.store(false, Ordering::Release);
    let proposal = prepare(&workspace, &script("write"), "sub", 10, &budget).unwrap();
    let moved = std::fs::rename(files.0.join("sub"), files.0.join("old-sub"));
    #[cfg(windows)]
    assert!(moved.is_err(), "held Windows directory must deny rename");
    #[cfg(target_os = "linux")]
    {
        moved.unwrap();
        std::fs::create_dir(files.0.join("sub")).unwrap();
        assert!(
            run(proposal, &workspace, &budget, &mut |_, _| {
                ControlFlow::Continue(())
            })
            .is_err()
        );
        assert!(!files.0.join("old-sub/command-result.txt").exists());
    }
    #[cfg(windows)]
    drop(proposal);
}
#[test]
fn fragmented_output_never_loses_unicode_or_executes_terminal_controls() {
    let mut capture = capture::Capture::default();
    let mut output = String::new();
    for byte in "a\r\ncafé\t\u{1b}]52;c;bad\u{7}\u{202e}".as_bytes() {
        output.push_str(&capture.push(&[*byte], false));
    }
    output.push_str(&capture.push(&[0xf0], true));
    assert_eq!(output, "a\ncafé    \\u{1b}]52;c;bad\\u{7}\\u{202e}\u{fffd}");
    assert_eq!(capture.tail, output);
}

#[test]
fn shell_output_arrives_before_the_command_finishes() {
    let files = Fixture::new();
    let workspace = Workspace::open(&files.0).unwrap();
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(20),
    };
    #[cfg(windows)]
    let script =
        "[Console]::WriteLine('first'); Start-Sleep -Seconds 2; [Console]::WriteLine('last')";
    #[cfg(not(windows))]
    let script = "printf 'first\\n'; sleep 2; printf 'last\\n'";
    let proposal = prepare(&workspace, script, ".", 15, &budget).unwrap();
    let started = Instant::now();
    let mut first = None;
    let mut last = None;
    let result = run(proposal, &workspace, &budget, &mut |_, text| {
        if text.contains("first") {
            first = Some(started.elapsed());
        }
        if text.contains("last") {
            last = Some(started.elapsed());
        }
        ControlFlow::Continue(())
    })
    .unwrap();
    assert!(result.success(), "{}", result.summary());
    assert!(last.unwrap().saturating_sub(first.unwrap()) > Duration::from_secs(1));
}

#[test]
fn cargo_test_stderr_and_failure_codes_reach_the_caller() {
    let files = Fixture::new();
    files.write("Cargo.toml", "[package]\nname = \"jecode_command_fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n[lib]\npath = \"code.rs\"\n");
    let workspace = Workspace::open(&files.0).unwrap();
    let cancelled = AtomicBool::new(false);
    for (source, expected) in [
        ("#[test] fn trial() { assert_eq!(2, 3); }", 101),
        ("#[test] fn trial() { assert_eq!(3, 3); }", 0),
    ] {
        files.write("code.rs", source);
        let budget = Budget {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(40),
        };
        let proposal = prepare(
            &workspace,
            "cargo test --offline --target-dir build",
            ".",
            30,
            &budget,
        )
        .unwrap();
        let result = run(proposal, &workspace, &budget, &mut |_, _| {
            ControlFlow::Continue(())
        })
        .unwrap();
        assert_eq!(result.stop, Stop::Exited, "{}", result.summary());
        assert_eq!(
            result.exit.unwrap().code,
            Some(expected),
            "{}\n{}\n{}",
            result.summary(),
            result.stdout,
            result.stderr
        );
        assert!(
            result.stderr.contains("Compiling") || result.stderr.contains("Finished"),
            "{}",
            result.stderr
        );
        assert!(result.stdout.contains(if expected == 0 {
            "1 passed"
        } else {
            "1 failed"
        }));
    }
}
