//! Opt-in, synthetic Windows measurements. Run with --ignored --nocapture.
use super::{Document, Handle, Store};
use crate::{
    cancel::Cancellation, effort::Effort, json::Value, process, redact::Redactor,
    test_support::Directory,
};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

// Get-Process is a Windows system tool. It samples this test process outside
// timed spans; working set is resident memory, not the OS filesystem cache.
fn memory_row(path: &Path, stage: &str, size: usize, repeat: usize) {
    let script = format!(
        "$p = Get-Process -Id {}; '{{0}},{{1}},{{2}}' -f $p.WorkingSet64, $p.PeakWorkingSet64, $p.PrivateMemorySize64",
        std::process::id()
    );
    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &script,
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let values = String::from_utf8(output.stdout).unwrap();
    assert_eq!(values.trim().split(',').count(), 3);
    let mut file = OpenOptions::new().append(true).open(path).unwrap();
    writeln!(file, "{stage},{size},{repeat},{}", values.trim()).unwrap();
}

fn message(role: &str, text: &str) -> Value {
    Value::object([
        ("role", Value::string(role)),
        ("content", Value::string(text)),
    ])
}

fn samples_path(name: &str) -> std::path::PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/measurements")
        .join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    path
}

fn row(path: &Path, operation: &str, size: usize, repeat: usize, elapsed: Duration, bytes: u64) {
    let mut file = OpenOptions::new().append(true).open(path).unwrap();
    writeln!(
        file,
        "{operation},{size},{repeat},{},{bytes}",
        elapsed.as_micros()
    )
    .unwrap();
}

fn timed<T>(work: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let result = work();
    (result, start.elapsed())
}

#[test]
#[ignore = "synthetic I/O benchmark; writes only under target"]
fn session_history_and_listing() {
    let path = samples_path("windows-sessions.csv");
    let memory = samples_path("windows-sessions-memory.csv");
    fs::write(
        &path,
        "operation,messages,repeat,elapsed_us,journal_bytes\n",
    )
    .unwrap();
    fs::write(
        &memory,
        "stage,messages,repeat,working_set_bytes,peak_working_set_bytes,private_bytes\n",
    )
    .unwrap();
    for size in [1_000, 5_000] {
        for repeat in 0..3 {
            let home = Directory::new();
            let directory = Directory::new();
            let store = Store::new(home.path().to_path_buf(), directory.path()).unwrap();
            let mut document = Document::new(
                directory.path().to_path_buf(),
                "fixture/model".into(),
                Effort::Default,
            )
            .unwrap();
            document
                .messages
                .push(message("system", "Synthetic system message"));
            let payload = "a".repeat(128);
            memory_row(&memory, "before_history", size, repeat);
            let history = (1..size)
                .map(|index| {
                    let role = if index % 2 == 1 { "user" } else { "assistant" };
                    message(role, &format!("{index}: {payload}"))
                })
                .collect::<Vec<_>>();
            let lease = store.create(&document.id).unwrap();
            let (_, append) = timed(|| {
                // One checkpoint per 50 messages keeps this bounded while retaining a
                // multi-record journal. The initial checkpoint includes the system text.
                lease.save(&document).unwrap();
                for chunk in history.chunks(50) {
                    document.messages.extend_from_slice(chunk);
                    lease.save(&document).unwrap();
                }
            });
            let journal = store.journal_path(&document.id);
            let bytes = fs::metadata(&journal).unwrap().len();
            assert_eq!(document.messages.len(), size);
            row(&path, "append_checkpoints", size, repeat, append, bytes);
            memory_row(&memory, "after_append", size, repeat);
            drop(lease);

            let (loaded, elapsed) = timed(|| store.fixture_load(&document.id).unwrap());
            assert_eq!(loaded.messages.len(), size);
            assert_eq!(loaded.messages, document.messages);
            row(&path, "load_journal", size, repeat, elapsed, bytes);
            memory_row(&memory, "after_load", size, repeat);

            let (_, elapsed) = timed(|| {
                let (handle, _) =
                    Handle::open(store.clone(), &document.id, Redactor::empty()).unwrap();
                assert_eq!(handle.snapshot().messages, document.messages);
            });
            row(&path, "resume_and_checkpoint", size, repeat, elapsed, bytes);
            memory_row(&memory, "after_resume", size, repeat);

            let (projected, elapsed) = timed(|| loaded.context.project(&loaded.messages, 0));
            assert_eq!(projected, loaded.messages);
            row(&path, "context_project", size, repeat, elapsed, bytes);
            let (estimated, elapsed) = timed(|| {
                loaded
                    .context
                    .estimate(&loaded.messages, 0, Default::default())
            });
            assert!(estimated > 0);
            row(&path, "context_estimate", size, repeat, elapsed, bytes);
            let (records, elapsed) = timed(|| loaded.records());
            assert_eq!(records.len(), size - 1);
            row(&path, "render_records", size, repeat, elapsed, bytes);
            memory_row(&memory, "after_records", size, repeat);

            let (listing, elapsed) = timed(|| store.list().unwrap());
            assert_eq!(listing.sessions.len(), 1);
            assert!(listing.warnings.is_empty());
            row(&path, "list_cached_one", size, repeat, elapsed, bytes);
            fs::remove_file(store.bucket.join(format!("{}.summary.json", document.id))).unwrap();
            let (listing, elapsed) = timed(|| store.list().unwrap());
            assert_eq!(listing.sessions.len(), 1);
            assert!(listing.warnings.is_empty());
            row(&path, "list_uncached_one", size, repeat, elapsed, bytes);
        }
    }
    println!("session measurements: {}", path.display());
}

#[test]
#[ignore = "synthetic process-start benchmark; writes only under target"]
fn windows_command_supervisor() {
    let path = samples_path("windows-process.csv");
    fs::write(&path, "operation,bytes,repeat,elapsed_us,exit_code\n").unwrap();
    let direct = || {
        let output = Command::new("hostname.exe").output().unwrap();
        assert!(output.status.success());
        assert!(!output.stdout.is_empty());
        output
    };
    let supervised = || {
        let output = process::run_observed(
            &mut Command::new("hostname.exe"),
            None,
            Some(Duration::from_secs(20)),
            4096,
            &Cancellation::default(),
            None,
            &mut |_| Ok(()),
        )
        .unwrap();
        assert_eq!(
            output.exit_code,
            Some(0),
            "supervisor stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!output.timed_out && !output.truncated && !output.cancelled);
        assert!(!output.stdout.is_empty());
        output
    };
    let reference = direct();
    let (cold, elapsed) = timed(supervised);
    assert_eq!(reference.stdout, cold.stdout);
    row(
        &path,
        "supervised_cold_cmd",
        cold.stdout_bytes,
        0,
        elapsed,
        0,
    );
    // After the first compilation, record three alternating warm pairs.
    for repeat in 0..3 {
        let (output, elapsed) = timed(direct);
        row(&path, "direct_cmd", output.stdout.len(), repeat, elapsed, 0);
        let (supervised_output, elapsed) = timed(supervised);
        assert_eq!(output.stdout, supervised_output.stdout);
        row(
            &path,
            "supervised_cmd",
            supervised_output.stdout_bytes,
            repeat,
            elapsed,
            0,
        );
    }
    println!("process measurements: {}", path.display());
}
