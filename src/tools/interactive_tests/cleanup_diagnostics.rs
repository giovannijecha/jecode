//! Failure-only observations for the interactive Bash cleanup contract.

use super::{Tools, Value, WAIT, execute, text, tools};
use crate::test_support::Directory;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn pid(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn heartbeat_len(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|entry| entry.len())
}

fn process_snapshot(
    root: &Path,
    watcher: Option<u32>,
    shell: Option<u32>,
    job: Option<u32>,
) -> String {
    let output = root.join("cleanup-ps-out");
    let error = root.join("cleanup-ps-err");
    let mut child = match Command::new("/bin/ps")
        .args([
            "-A", "-o", "pid=", "-o", "ppid=", "-o", "pgid=", "-o", "stat=", "-o", "comm=",
        ])
        .stdout(Stdio::from(File::create(&output).unwrap()))
        .stderr(Stdio::from(File::create(&error).unwrap()))
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return format!("ps spawn failed: {error}"),
    };
    let deadline = Instant::now() + Duration::from_secs(1);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return "ps timed out after 1 s (killed)".into();
            }
            Err(error) => break Err(error),
        }
    };
    let Ok(status) = status else {
        return format!("ps wait failed: {}", status.unwrap_err());
    };
    if !status.success() {
        let mut stderr = Vec::new();
        if let Ok(file) = File::open(error) {
            let _ = file.take(512).read_to_end(&mut stderr);
        }
        return format!(
            "ps exited with {status}; stderr: {:?}",
            String::from_utf8_lossy(&stderr)
        );
    }
    let Ok(file) = File::open(output) else {
        return "ps output unavailable".into();
    };
    let mut rows = Vec::new();
    for line in BufReader::new(file.take(1024 * 1024))
        .lines()
        .map_while(Result::ok)
    {
        let mut fields = line.split_whitespace();
        let process = fields.next().and_then(|value| value.parse::<u32>().ok());
        let parent = fields.next().and_then(|value| value.parse::<u32>().ok());
        let command = line.split_whitespace().last();
        let selected = process.is_some_and(|process| {
            Some(process) == watcher || Some(process) == shell || Some(process) == job
        });
        let watcher_ps = watcher.is_some()
            && parent == watcher
            && command.is_some_and(|command| command == "ps" || command.ends_with("/ps"));
        if selected || watcher_ps {
            rows.push(line);
            if rows.len() == 8 {
                break;
            }
        }
    }
    format!("ps matched: {rows:?}")
}

fn eof_offsets(eof: Option<[Option<Instant>; 2]>, origin: Instant) -> String {
    let Some([stdout, stderr]) = eof else {
        return "capture entry unavailable".into();
    };
    let offset = |at: Option<Instant>| match at {
        Some(at) if at >= origin => format!("+{} ms", (at - origin).as_millis()),
        Some(at) => format!("-{} ms", (origin - at).as_millis()),
        None => "open or not EOF".into(),
    };
    format!("stdout={}, stderr={}", offset(stdout), offset(stderr))
}

pub(super) fn failure(
    tools: &Tools,
    id: &str,
    result: &Value,
    pid_paths: [&Path; 3],
    heartbeat: &Path,
    root: &Path,
) -> String {
    let failed_at = Instant::now();
    let [watcher_path, shell_path, job_path] = pid_paths;
    let watcher = pid(watcher_path);
    let shell = pid(shell_path);
    let job = pid(job_path);
    let heartbeat_at_failure = heartbeat_len(heartbeat);
    let eof_at_failure = tools.processes.capture_eof(id);
    let processes = process_snapshot(root, watcher, shell, job);
    let deadline = failed_at + Duration::from_secs(1);
    loop {
        let eof = tools.processes.capture_eof(id);
        if eof.is_some_and(|streams| streams.into_iter().all(|at| at.is_some()))
            || Instant::now() >= deadline
        {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let eof_later = tools.processes.capture_eof(id);
    let heartbeat_later = heartbeat_len(heartbeat);
    format!(
        "stop error={:?}; watcher={watcher:?}, shell={shell:?}, job={job:?}; {processes}; heartbeat bytes at failure={heartbeat_at_failure:?}, after probe={heartbeat_later:?}; capture EOF at failure: {}; capture EOF after probe: {}; probe elapsed={} ms",
        result.get("error").and_then(Value::as_str),
        eof_offsets(eof_at_failure, failed_at),
        eof_offsets(eof_later, failed_at),
        failed_at.elapsed().as_millis(),
    )
}

#[test]
fn interactive_bash_job_groups_end_on_stop_and_conversation_change() {
    for stop_conversation in [false, true] {
        let root = Directory::new();
        let tools = tools(&root);
        let heartbeat = root.path().join("job-heartbeat");
        let shell_pid = root.path().join("shell-pid");
        let job_pid = root.path().join("job-pid");
        let watcher_pid = root.path().join("watcher-pid");
        let script = r#"printf '%s' "$$" > "$JECODE_TTY_SHELL"; printf '%s' "$PPID" > "$JECODE_TTY_WATCHER"; while :; do printf x >> "$JECODE_TTY_HEARTBEAT"; sleep 0.02; done & printf '%s' "$!" > "$JECODE_TTY_JOB"; wait"#;
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
                    (
                        "JECODE_TTY_WATCHER",
                        Value::string(watcher_pid.to_string_lossy()),
                    ),
                ]),
            )],
        );
        let id = text(&started, "process_id").to_owned();
        let beginning = Instant::now();
        while fs::metadata(&heartbeat).map_or(0, |entry| entry.len()) < 2 {
            assert!(
                beginning.elapsed() < WAIT,
                "background job did not start: {started:?}"
            );
            thread::sleep(Duration::from_millis(20));
        }
        let shell = fs::read_to_string(&shell_pid).unwrap();
        let job = fs::read_to_string(&job_pid).unwrap();
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
            let stopped = tools.execute(
                "process",
                &Value::object([
                    ("action", Value::string("stop")),
                    ("process_id", Value::string(&id)),
                    ("wait_ms", Value::number(200)),
                ])
                .encode(),
            );
            assert_eq!(
                text(&stopped, "status"),
                "stopped",
                "{}",
                failure(
                    &tools,
                    &id,
                    &stopped,
                    [&watcher_pid, &shell_pid, &job_pid],
                    &heartbeat,
                    root.path(),
                )
            );
        }
        let length = fs::metadata(&heartbeat).unwrap().len();
        thread::sleep(Duration::from_millis(300));
        assert_eq!(
            fs::metadata(&heartbeat).unwrap().len(),
            length,
            "interactive Bash job survived terminal cleanup"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_snapshot_reports_only_the_requested_process() {
        let root = crate::test_support::Directory::new();
        let current = std::process::id();
        let report = process_snapshot(root.path(), None, Some(current), None);
        assert!(report.starts_with("ps matched: [\""), "{report}");
        assert!(report.contains(&current.to_string()), "{report}");
        assert!(!report.contains("\", \""), "unrelated ps rows: {report}");
        assert!(report.len() < 512, "unbounded ps report: {report}");
    }
}
