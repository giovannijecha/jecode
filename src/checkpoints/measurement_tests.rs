//! Opt-in measurements of integrated checkpoint workflows on isolated synthetic
//! workspaces. Run them against the optimized build:
//!
//! ```text
//! cargo test --locked --offline --release checkpoints::measurement_tests::workflows -- --exact --ignored --nocapture --test-threads=1
//! ```
//!
//! `JECODE_CHECKPOINT_MEASURE_OUT` names the output directory, by default
//! `.tmp/checkpoint-measurements/<os>`; `JECODE_CHECKPOINT_MEASURE_QUICK=1`
//! takes a tenth of the samples to check the harness itself. Set
//! `JECODE_CHECKPOINT_MEASURE_ONLY` to comma-separated workflow names to rerun
//! capture, editing, recovery, storage or sessions independently. The `large`
//! workflow creates a 264 MiB workspace and 50,010 files; select it explicitly.
use super::store::Counters;
use super::workspace::{self, Options, Readiness, Watching, Workspace};
use crate::test_support::Directory;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod capture;
mod editing;
mod environment;
mod large;
mod recovery;
mod sessions;
mod storage;
mod streaming;

const OUT: &str = "JECODE_CHECKPOINT_MEASURE_OUT";
const QUICK: &str = "JECODE_CHECKPOINT_MEASURE_QUICK";
const ONLY: &str = "JECODE_CHECKPOINT_MEASURE_ONLY";

#[test]
#[ignore = "opt-in measurement; see the module documentation"]
fn workflows() {
    let output = std::env::var_os(OUT).map_or_else(
        || {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(".tmp/checkpoint-measurements")
                .join(std::env::consts::OS)
        },
        PathBuf::from,
    );
    let mut recorder = Recorder {
        quick: std::env::var_os(QUICK).is_some(),
        ..Recorder::default()
    };
    let only = std::env::var(ONLY).ok();
    let workflows = selected(only.as_deref()).unwrap();
    let started = Instant::now();
    for (name, run) in [
        ("capture", capture::run as fn(&mut Recorder)),
        ("editing", editing::run),
        ("recovery", recovery::run),
        ("storage", storage::run),
        ("sessions", sessions::run),
        ("large", large::run),
        ("streaming", streaming::run),
    ] {
        if workflows.contains(&name) {
            run(&mut recorder);
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    let environment = environment::identity(&output, recorder.quick, elapsed);
    recorder.write(&output, &environment).unwrap();
}

fn selected(value: Option<&str>) -> Result<Vec<&str>, String> {
    let default = ["capture", "editing", "recovery", "storage", "sessions"];
    let all = [
        "capture",
        "editing",
        "recovery",
        "storage",
        "sessions",
        "large",
        "streaming",
    ];
    let Some(value) = value else {
        return Ok(default.to_vec());
    };
    let chosen: Vec<&str> = value.split(',').map(str::trim).collect();
    if chosen.iter().any(|name| !all.contains(name)) {
        return Err(format!(
            "Invalid measurement workflow; use {}",
            all.join(", ")
        ));
    }
    Ok(chosen)
}

#[test]
fn workflow_selection_rejects_unknown_or_empty_measurements() {
    assert_eq!(selected(None).unwrap().len(), 5);
    assert_eq!(
        selected(Some("editing, recovery")).unwrap(),
        ["editing", "recovery"]
    );
    assert!(selected(Some("")).is_err());
    assert!(selected(Some("edting")).is_err());
    assert_eq!(selected(Some("large")).unwrap(), ["large"]);
}

#[derive(Default)]
pub(super) struct Recorder {
    quick: bool,
    series: Vec<Series>,
    metrics: Vec<[String; 5]>,
}

struct Series {
    key: [String; 3],
    note: String,
    samples: Vec<f64>,
}

impl Recorder {
    /// The sample count for a workflow, reduced in a quick run.
    pub fn count(&self, normal: usize) -> usize {
        if self.quick {
            (normal / 10).max(3)
        } else {
            normal
        }
    }

    pub fn add(&mut self, key: [&str; 3], note: &str, samples: Vec<f64>) {
        let stats = Stats::of(&samples);
        println!(
            "{:<12} {:<44} {:<12} n={:<4} median {:>10.3} ms  min {:>10.3}  max {:>10.3}",
            key[0], key[1], key[2], stats.n, stats.median, stats.min, stats.max
        );
        self.series.push(Series {
            key: key.map(str::to_string),
            note: note.into(),
            samples,
        });
    }

    /// Records alternating-order pairs `(baseline, checkpoint)` and their difference.
    pub fn paired(&mut self, workflow: &str, case: &str, note: &str, pairs: &[(f64, f64)]) {
        let baseline = pairs.iter().map(|pair| pair.0).collect();
        let checkpoint = pairs.iter().map(|pair| pair.1).collect();
        let added = pairs.iter().map(|pair| pair.1 - pair.0).collect();
        self.add([workflow, case, "baseline"], note, baseline);
        self.add([workflow, case, "checkpoint"], note, checkpoint);
        self.add(
            [workflow, case, "added"],
            "paired checkpoint minus baseline",
            added,
        );
    }

    pub fn metric(&mut self, workflow: &str, case: &str, name: &str, value: f64, unit: &str) {
        self.metrics
            .push([workflow, case, name, &format!("{value}"), unit].map(str::to_string));
    }

    /// Archive work per operation between two counter snapshots.
    pub fn counters(
        &mut self,
        key: [&str; 2],
        before: Counters,
        after: Counters,
        operations: usize,
    ) {
        for ((name, start), (_, end)) in fields(&before).into_iter().zip(fields(&after)) {
            let per = (end - start) as f64 / operations.max(1) as f64;
            self.metric(
                key[0],
                key[1],
                &format!("{name} per operation"),
                per,
                "count",
            );
        }
    }

    fn write(&self, directory: &Path, environment: &str) -> std::io::Result<()> {
        fs::create_dir_all(directory)?;
        let mut samples = String::from("workflow,case,variant,sample,ms\n");
        let mut summary =
            String::from("workflow,case,variant,n,median_ms,min_ms,max_ms,p95_ms,note\n");
        for series in &self.series {
            let key = series
                .key
                .iter()
                .map(|field| csv(field))
                .collect::<Vec<_>>();
            for (index, sample) in series.samples.iter().enumerate() {
                let _ = writeln!(samples, "{},{index},{sample:.6}", key.join(","));
            }
            let stats = Stats::of(&series.samples);
            let p95 = stats.p95.map_or(String::new(), |p95| format!("{p95:.6}"));
            let _ = writeln!(
                summary,
                "{},{},{:.6},{:.6},{:.6},{p95},{}",
                key.join(","),
                stats.n,
                stats.median,
                stats.min,
                stats.max,
                csv(&series.note)
            );
        }
        let mut metrics = String::from("workflow,case,name,value,unit\n");
        for metric in &self.metrics {
            let row = metric.iter().map(|field| csv(field)).collect::<Vec<_>>();
            let _ = writeln!(metrics, "{}", row.join(","));
        }
        fs::write(directory.join("samples.csv"), samples)?;
        fs::write(directory.join("summary.csv"), summary)?;
        fs::write(directory.join("metrics.csv"), metrics)?;
        fs::write(directory.join("environment.json"), environment)?;
        println!("measurements written to {}", directory.display());
        Ok(())
    }
}

fn fields(counters: &Counters) -> [(&'static str, u64); 10] {
    [
        ("transactions", counters.transactions),
        ("flushes", counters.flushes),
        ("frames", counters.frames),
        ("bytes written", counters.bytes_written),
        ("objects full", counters.objects_full),
        ("objects delta", counters.objects_delta),
        ("objects reused", counters.objects_reused),
        ("files read", counters.files_read),
        ("bytes read", counters.bytes_read),
        ("scans", counters.scans),
    ]
}

fn csv(field: &str) -> String {
    if field.contains([',', '"', '\n']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.into()
    }
}

struct Stats {
    n: usize,
    median: f64,
    min: f64,
    max: f64,
    /// Only reported with at least 100 samples.
    p95: Option<f64>,
}

impl Stats {
    fn of(samples: &[f64]) -> Self {
        let mut sorted = samples.to_vec();
        sorted.sort_by(f64::total_cmp);
        let n = sorted.len();
        let median = match n {
            0 => f64::NAN,
            _ if n % 2 == 1 => sorted[n / 2],
            _ => (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0,
        };
        Self {
            n,
            median,
            min: sorted.first().copied().unwrap_or(f64::NAN),
            max: sorted.last().copied().unwrap_or(f64::NAN),
            p95: (n >= 100).then(|| sorted[(n * 95).div_ceil(100) - 1]),
        }
    }
}

/// Milliseconds spent in `work`.
pub(super) fn timed<T>(work: impl FnOnce() -> T) -> (T, f64) {
    let started = Instant::now();
    let value = work();
    (value, started.elapsed().as_secs_f64() * 1000.0)
}

/// Deterministic source-like text of exactly `bytes` bytes.
pub(super) fn text(seed: u64, bytes: usize) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    let mut out = String::with_capacity(bytes + 96);
    let mut line = 0;
    while out.len() < bytes {
        state = next(state);
        let indent = (state % 3) as usize * 4;
        let _ = writeln!(
            out,
            "{:indent$}let value_{line} = compute({}, \"{:016x}\");",
            "",
            state % 1000,
            state
        );
        line += 1;
    }
    out.truncate(bytes);
    out.into_bytes()
}

pub(super) fn binary(seed: u64, bytes: usize) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x2545_f491_4f6c_dd1d) | 1;
    (0..bytes)
        .map(|_| {
            state = next(state);
            (state >> 24) as u8
        })
        .collect()
}

pub(super) fn next(mut state: u64) -> u64 {
    state ^= state << 13;
    state ^= state >> 7;
    state ^ (state << 17)
}

/// An isolated workspace and archive home.
pub(super) struct Tree {
    pub home: Directory,
    pub root: Directory,
    pub files: Vec<String>,
}

impl Tree {
    /// `files` source-like files of 0.5–4 KiB, 100 per directory.
    pub fn new(files: usize, seed: u64) -> Self {
        let tree = Self {
            home: Directory::new(),
            root: Directory::new(),
            files: (0..files)
                .map(|index| format!("src/d{:03}/f{index:05}.rs", index / 100))
                .collect(),
        };
        let mut state = seed | 1;
        for (index, relative) in tree.files.iter().enumerate() {
            state = next(state);
            let size = 512 + (state % 3584) as usize;
            tree.write(relative, &text(seed + index as u64, size));
        }
        tree
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        self.root.path().join(relative)
    }

    pub fn write(&self, relative: &str, bytes: &[u8]) {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    /// Opens the workspace as the TUI does and waits for its baseline.
    pub fn open(&self) -> Arc<Workspace> {
        self.open_with(Options::interactive())
    }

    pub fn open_with(&self, options: Options) -> Arc<Workspace> {
        let workspace = workspace::open(self.home.path(), self.root.path(), options).unwrap();
        if !options.background {
            workspace.initialize().unwrap();
        }
        ready(&workspace);
        workspace
    }
}

/// Waits until the baseline is complete and notifications have settled.
pub(super) fn ready(workspace: &Workspace) {
    let started = Instant::now();
    loop {
        let health = workspace.signals.lock();
        let settled = !matches!(health.watching, Watching::Starting);
        match &health.readiness {
            Readiness::Ready if settled => return,
            Readiness::Failed(error) => panic!("checkpoint capture failed: {error}"),
            _ => {}
        }
        assert!(
            started.elapsed() < Duration::from_secs(600),
            "the baseline did not finish"
        );
        let _ = workspace
            .signals
            .changed
            .wait_timeout(health, Duration::from_millis(5));
    }
}

pub(super) fn counters(workspace: &Workspace) -> Counters {
    workspace.store().counters
}

/// Total bytes of the private archive files. Windows directory listings keep
/// a stale size for files still open, so each file is queried directly.
pub(super) fn archive_bytes(directory: &Path) -> u64 {
    let mut total = 0;
    let mut pending = vec![directory.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            match fs::symlink_metadata(entry.path()) {
                Ok(metadata) if metadata.is_dir() => pending.push(entry.path()),
                Ok(metadata) => total += metadata.len(),
                Err(_) => {}
            }
        }
    }
    total
}
