//! Concurrent sessions as separate processes sharing one archive. Children are
//! this test binary re-entered through an ignored test that acts only when asked.
use super::capture::close;
use super::*;
use crate::checkpoints::hash;
use crate::checkpoints::mutation::Protection;
use crate::checkpoints::record::State;
use std::process::{Command, Stdio};

const MODE: &str = "JECODE_CHECKPOINT_MEASURE_CHILD";
const HOME: &str = "JECODE_CHECKPOINT_MEASURE_HOME";
const ROOT: &str = "JECODE_CHECKPOINT_MEASURE_ROOT";
const CONTROL: &str = "JECODE_CHECKPOINT_MEASURE_CONTROL";
const INDEX: &str = "JECODE_CHECKPOINT_MEASURE_INDEX";
const WRITES: &str = "JECODE_CHECKPOINT_MEASURE_WRITES";

pub fn run(recorder: &mut Recorder) {
    #[cfg(windows)]
    cold_helper(recorder);
    let writes = recorder.count(100);
    for shared in [false, true] {
        let mut single = f64::NAN;
        for sessions in [1, 2, 4] {
            let median = concurrent(recorder, sessions, shared, writes);
            if sessions == 1 {
                single = median;
            }
            let case = case(sessions, shared);
            recorder.metric(
                "sessions",
                &case,
                "added median wait",
                median - single,
                "ms",
            );
        }
    }
}

fn case(sessions: usize, shared: bool) -> String {
    let paths = if shared {
        "shared paths"
    } else {
        "distinct paths"
    };
    format!("{sessions} session(s), {paths}")
}

fn spawn(
    mode: &str,
    tree: &Tree,
    control: &Path,
    index: usize,
    writes: usize,
) -> std::process::Child {
    Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "checkpoints::measurement_tests::sessions::child",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(MODE, mode)
        .env(HOME, tree.home.path())
        .env(ROOT, tree.root.path())
        .env(CONTROL, control)
        .env(INDEX, index.to_string())
        .env(WRITES, writes.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

/// The numbers a child printed after `prefix`. The first line it prints
/// follows the test harness's unterminated `test NAME ... ` line.
fn values(output: &str, prefix: &str) -> Vec<f64> {
    output
        .lines()
        .filter_map(|line| line.split_once(prefix).map(|(_, rest)| rest))
        .flat_map(|rest| {
            rest.split_whitespace()
                .map(|value| value.parse::<f64>().unwrap())
        })
        .collect()
}

fn wait(child: std::process::Child) -> String {
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

/// Each session writes 4 KiB files through the mutation API, all starting together.
fn concurrent(recorder: &mut Recorder, sessions: usize, shared: bool, writes: usize) -> f64 {
    let tree = Tree::new(1_000, 60);
    close(tree.open_with(Options::offline()));
    let control = Directory::new();
    let mode = if shared { "shared" } else { "distinct" };
    let children: Vec<_> = (0..sessions)
        .map(|index| spawn(mode, &tree, control.path(), index, writes))
        .collect();
    let started = Instant::now();
    while (0..sessions).any(|index| !control.path().join(format!("ready-{index}")).exists()) {
        assert!(
            started.elapsed() < Duration::from_secs(300),
            "the sessions did not start"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    fs::write(control.path().join("go"), b"").unwrap();
    let outputs: Vec<String> = children.into_iter().map(wait).collect();
    let latencies: Vec<f64> = outputs
        .iter()
        .flat_map(|output| values(output, "SAMPLES "))
        .collect();
    let elapsed = outputs
        .iter()
        .flat_map(|output| values(output, "ELAPSED "))
        .fold(0.0, f64::max);
    let errors: f64 = outputs
        .iter()
        .flat_map(|output| values(output, "ERRORS "))
        .sum();
    let case = case(sessions, shared);
    let total = (sessions * writes) as f64;
    recorder.metric(
        "sessions",
        &case,
        "throughput",
        total / (elapsed / 1000.0),
        "writes per second",
    );
    recorder.metric(
        "sessions",
        &case,
        "unprotected or failed writes",
        errors,
        "count",
    );

    // Every file on disk matches the archive, checked by a fresh process store.
    let workspace = tree.open_with(Options::offline());
    let store = workspace.store();
    let mut checked = 0;
    for index in 0..sessions {
        for slot in 0..10 {
            let relative = path(shared, index, slot);
            let bytes = fs::read(tree.path(&relative)).unwrap();
            let state = store.index.current(&relative).map(|version| &version.state);
            assert!(
                matches!(state, Some(State::File { object, digest, .. })
                    if *digest == hash::digest(&bytes)
                        && store.bytes(*object).is_ok_and(|saved| saved[..] == bytes[..])),
                "{relative}"
            );
            checked += 1;
        }
    }
    drop(store);
    close(workspace);
    recorder.metric(
        "sessions",
        &case,
        "paths verified byte-exact",
        f64::from(checked),
        "count",
    );
    let stats_median = {
        let mut sorted = latencies.clone();
        sorted.sort_by(f64::total_cmp);
        sorted.get(sorted.len() / 2).copied().unwrap_or(f64::NAN)
    };
    recorder.add(
        ["sessions", &case, "per write"],
        "separate processes, product options, 4 KiB writes",
        latencies,
    );
    stats_median
}

fn path(shared: bool, index: usize, slot: usize) -> String {
    if shared {
        format!("c/shared/f{slot}.txt")
    } else {
        format!("c/s{index}/f{slot}.txt")
    }
}

/// Entry point of a child process; does nothing in a normal test run.
#[test]
#[ignore = "run by the checkpoint measurements"]
fn child() {
    let Ok(mode) = std::env::var(MODE) else {
        return;
    };
    let variable = |name: &str| std::env::var(name).unwrap();
    let (home, root) = (PathBuf::from(variable(HOME)), PathBuf::from(variable(ROOT)));
    if mode == "helper" {
        let started = Instant::now();
        let workspace = workspace::open(&home, &root, Options::interactive()).unwrap();
        workspace.initialize().unwrap();
        while workspace.watching() != Watching::Active {
            assert!(
                started.elapsed() < Duration::from_secs(300),
                "{:?}",
                workspace.watching()
            );
            std::thread::sleep(Duration::from_micros(200));
        }
        println!("SAMPLES {}", started.elapsed().as_secs_f64() * 1000.0);
        return;
    }
    let control = PathBuf::from(variable(CONTROL));
    let index: usize = variable(INDEX).parse().unwrap();
    let writes: usize = variable(WRITES).parse().unwrap();
    let workspace = workspace::open(&home, &root, Options::interactive()).unwrap();
    ready(&workspace);
    fs::write(control.join(format!("ready-{index}")), b"").unwrap();
    while !control.join("go").exists() {
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut samples = Vec::with_capacity(writes);
    let mut errors = 0;
    let started = Instant::now();
    for write in 0..writes {
        let relative = path(mode == "shared", index, write % 10);
        let bytes = text((index * 100_000 + write) as u64, 4096);
        let target = root.join(&relative);
        let (protection, time) = timed(|| {
            let mut mutation = workspace.mutation(&format!("session-{index}"), "write", &target);
            mutation.prepare(&bytes);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(&target, &bytes).unwrap();
            mutation.finish(true)
        });
        if !matches!(protection, Protection::Protected { .. }) {
            errors += 1;
        }
        samples.push(time.to_string());
    }
    println!("ELAPSED {}", started.elapsed().as_secs_f64() * 1000.0);
    println!("ERRORS {errors}");
    println!("SAMPLES {}", samples.join(" "));
}

/// The first helper start of a fresh process compiles the helper.
#[cfg(windows)]
fn cold_helper(recorder: &mut Recorder) {
    let samples = recorder.count(3).min(3);
    let times = (0..samples)
        .map(|_| {
            let tree = Tree::new(100, 61);
            let control = Directory::new();
            let output = wait(spawn("helper", &tree, control.path(), 0, 0));
            values(&output, "SAMPLES ")[0]
        })
        .collect();
    recorder.add(
        [
            "notifications",
            "explicit full-scan helper start, new process (compiles)",
            "total",
        ],
        "explicit scan starts the watcher; ordinary on-demand startup does not; fresh helper cache per test process",
        times,
    );
}
