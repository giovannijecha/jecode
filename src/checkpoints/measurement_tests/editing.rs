//! Tool calls with and without checkpoints, as alternating-order pairs on two
//! identical workspaces: file writes and edits, binary mutations and Bash.
use super::*;
use crate::checkpoints::hash;
use crate::checkpoints::record::State;
use crate::checkpoints::session::Session;
use crate::json::Value;
use crate::tools::Tools;

/// Untimed pairs before each paired series.
pub(super) const WARMUPS: usize = 2;

pub fn run(recorder: &mut Recorder) {
    let bench = Bench::new(1_000);
    files(recorder, &bench);
    bash(recorder, &bench);
    background(recorder, &bench);
}

/// Identical workspaces; only the second has checkpoints.
pub(super) struct Bench {
    pub trees: [Tree; 2],
    pub tools: [Tools; 2],
    pub session: Arc<Session>,
}

impl Bench {
    pub fn new(files: usize) -> Self {
        let trees = [Tree::new(files, 11), Tree::new(files, 11)];
        let mut tools = trees.each_ref().map(|tree| {
            let mut tools = Tools::new(tree.root.path()).unwrap();
            let logs = tree.home.path().join("logs");
            tools.configure_output(logs, crate::redact::Redactor::empty());
            tools
        });
        let session = Session::open(trees[1].home.path(), tools[1].root());
        session.begin("measure", None);
        ready(session.workspace().unwrap());
        tools[1].configure_checkpoints(Arc::clone(&session));
        Self {
            trees,
            tools,
            session,
        }
    }

    pub fn workspace(&self) -> &Arc<Workspace> {
        self.session.workspace().unwrap()
    }

    /// Creates a directory on both sides; the write tool needs its parent.
    pub fn directory(&self, relative: &str) {
        for tree in &self.trees {
            fs::create_dir_all(tree.path(relative)).unwrap();
        }
    }

    /// Writes `bytes` on both sides outside the archive's operations and
    /// brings the archive up to date.
    pub fn prepare(&self, relative: &str, bytes: &[u8]) {
        for tree in &self.trees {
            tree.write(relative, bytes);
        }
        self.workspace()
            .mark("turn", "prepare", "measure", false, false)
            .unwrap();
    }

    /// Runs `call` on both sides, alternating which goes first, each pair
    /// after an untimed `pause`.
    pub fn pairs(
        &self,
        samples: std::ops::Range<usize>,
        pause: Duration,
        mut call: impl FnMut(&Tools, &Tree, usize),
    ) -> Vec<(f64, f64)> {
        samples
            .map(|sample| {
                std::thread::sleep(pause);
                let mut times = [0.0; 2];
                let order = if sample % 2 == 0 { [0, 1] } else { [1, 0] };
                for side in order {
                    let tools = &self.tools[side];
                    let tree = &self.trees[side];
                    times[side] = timed(|| call(tools, tree, sample)).1;
                }
                (times[0], times[1])
            })
            .collect()
    }

    /// Records paired samples and the archive work they caused, after
    /// untimed warm-up pairs that continue the same sample sequence.
    pub fn measure(
        &self,
        recorder: &mut Recorder,
        key: [&str; 2],
        note: &str,
        samples: usize,
        pause: Duration,
        mut call: impl FnMut(&Tools, &Tree, usize),
    ) {
        self.pairs(0..WARMUPS, pause, &mut call);
        let before = counters(self.workspace());
        let pairs = self.pairs(WARMUPS..WARMUPS + samples, pause, call);
        recorder.counters(key, before, counters(self.workspace()), samples);
        recorder.paired(key[0], key[1], note, &pairs);
    }

    /// Asserts that the archive holds exactly the bytes now on disk.
    pub fn captured(&self, relative: &str) {
        let bytes = fs::read(self.trees[1].path(relative)).unwrap();
        let store = self.workspace().store();
        let state = store.index.current(relative).map(|version| &version.state);
        assert!(
            matches!(state, Some(State::File { digest, .. }) if *digest == hash::digest(&bytes)),
            "{relative} differs from the archive"
        );
    }
}

pub(super) fn call(tools: &Tools, name: &str, fields: &[(&str, Value)]) -> Value {
    let arguments = Value::Object(
        fields
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect(),
    );
    let result = tools.execute(name, &arguments.encode());
    assert!(result.get("error").is_none(), "{name}: {result:?}");
    result
}

fn write(tools: &Tools, relative: &str, content: &str) {
    let fields = [
        ("path", Value::string(relative)),
        ("content", Value::string(content)),
    ];
    let result = call(tools, "write", &fields);
    if tools.checkpoints().is_some() {
        let status = result
            .get("checkpoint")
            .and_then(|value| value.get("status"));
        assert_eq!(status.and_then(Value::as_str), Some("saved"), "{result:?}");
    }
}

/// Replaces marker `sample` with `sample + 1`, so every edit is new content.
fn edit(tools: &Tools, relative: &str, sample: usize) {
    let marker = |value: usize| format!("let marker = {value};");
    let fields = [
        ("path", Value::string(relative)),
        ("old_text", Value::string(marker(sample))),
        ("new_text", Value::string(marker(sample + 1))),
    ];
    call(tools, "edit", &fields);
}

fn utf8(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap()
}

/// The untimed pause before each pair of calls on a file of `bytes`. Opening
/// a file just written waits for on-access antivirus scans, which would fall
/// on whichever side wrote last; between the model's calls they have ended.
pub(super) fn pause(bytes: usize) -> Duration {
    Duration::from_millis(100 + bytes as u64 / 1024)
}

/// Distinct content for each warm-up and sample, generated outside the timer,
/// so every call stores bytes the archive does not hold yet.
fn fresh(seed: u64, bytes: usize, samples: usize) -> Vec<String> {
    (0..(WARMUPS + samples) as u64)
        .map(|sample| utf8(text(seed + sample, bytes)))
        .collect()
}

fn files(recorder: &mut Recorder, bench: &Bench) {
    let workflow = "write/edit";
    let note = "tool call through Tools::execute; 1,000-file workspace; new content in every call; untimed pause of 100 ms plus 1 ms per KiB before each pair";
    bench.directory("w");
    let samples = recorder.count(200);
    let small = fresh(1 << 20, 4096, samples);
    bench.measure(
        recorder,
        [workflow, "write 4 KiB (replace)"],
        note,
        samples,
        pause(4096),
        |tools, _, sample| {
            write(tools, "w/four.txt", &small[sample]);
        },
    );
    bench.captured("w/four.txt");
    let samples = recorder.count(100);
    let small = fresh(2 << 20, 4096, samples);
    bench.measure(
        recorder,
        [workflow, "write 4 KiB (create)"],
        note,
        samples,
        pause(4096),
        |tools, _, sample| {
            write(tools, &format!("w/new{sample:04}.txt"), &small[sample]);
        },
    );

    let mut source = text(3, 40 * 1024);
    source.extend_from_slice(b"\nlet marker = 0;\n");
    bench.prepare("e/text.rs", &source);
    let samples = recorder.count(200);
    bench.measure(
        recorder,
        [workflow, "edit one line of 40 KiB text"],
        note,
        samples,
        pause(source.len()),
        |tools, _, sample| {
            edit(tools, "e/text.rs", sample);
        },
    );
    bench.captured("e/text.rs");

    let mut large = text(4, 1 << 20);
    large.extend_from_slice(b"\nlet marker = 0;\n");
    bench.prepare("e/large.rs", &large);
    let samples = recorder.count(50);
    bench.measure(
        recorder,
        [workflow, "edit one line of 1 MiB text"],
        note,
        samples,
        pause(large.len()),
        |tools, _, sample| {
            edit(tools, "e/large.rs", sample);
        },
    );
    bench.captured("e/large.rs");
    let rewrites = fresh(3 << 20, 1 << 20, samples);
    bench.measure(
        recorder,
        [workflow, "rewrite 1 MiB text"],
        note,
        samples,
        pause(large.len()),
        |tools, _, sample| {
            write(tools, "e/large.rs", &rewrites[sample]);
        },
    );
    bench.captured("e/large.rs");
    // The second edit reads the file the first one just wrote.
    bench.prepare("e/twice.rs", &large);
    bench.measure(
        recorder,
        [workflow, "edit one line of 1 MiB text twice in a row"],
        "both edits timed together; untimed pause before each pair",
        samples,
        pause(large.len()),
        |tools, _, sample| {
            edit(tools, "e/twice.rs", 2 * sample);
            edit(tools, "e/twice.rs", 2 * sample + 1);
        },
    );
    bench.captured("e/twice.rs");
    next_point(recorder, bench, &large, samples);

    // The file tools take text; binary content goes through the same mutation.
    let blobs: Vec<Vec<u8>> = (0..(WARMUPS + samples) as u64)
        .map(|sample| binary((4 << 20) + sample, 256 * 1024))
        .collect();
    bench.directory("b");
    let workspace = bench.workspace();
    bench.measure(
        recorder,
        [workflow, "write 256 KiB binary"],
        "fs::write versus the mutation API the tools use; new content in every call; untimed pause before each pair",
        samples,
        pause(256 * 1024),
        |tools, tree, sample| {
            let path = tree.path("b/blob.bin");
            let bytes = &blobs[sample];
            if tools.checkpoints().is_none() {
                return fs::write(&path, bytes).unwrap();
            }
            let mut mutation = workspace.mutation("measure", "write", &path);
            mutation.prepare(bytes);
            fs::write(&path, bytes).unwrap();
            mutation.finish(true);
        },
    );
    bench.captured("b/blob.bin");
}

/// A point immediately after an edit, as when a Bash command follows it:
/// the point rereads the file the edit just wrote.
fn next_point(recorder: &mut Recorder, bench: &Bench, large: &[u8], samples: usize) {
    bench.prepare("e/next.rs", large);
    let workspace = bench.workspace();
    let mut times = Vec::new();
    for sample in 0..WARMUPS + samples {
        std::thread::sleep(pause(large.len()));
        edit(&bench.tools[1], "e/next.rs", sample);
        let (seq, ms) = timed(|| workspace.mark("turn", "measure", "measure", false, true));
        seq.unwrap();
        if sample >= WARMUPS {
            times.push(ms);
        }
    }
    bench.captured("e/next.rs");
    recorder.add(
        [
            "points",
            "1000 files: right after editing 1 MiB text",
            "product options",
        ],
        "the edit is untimed; compare with a reused point",
        times,
    );
}

/// A named Bash command for each sample.
type Case = (&'static str, fn(usize) -> String);

fn bash(recorder: &mut Recorder, bench: &Bench) {
    let workflow = "bash";
    let note = "foreground bash tool call; process start dominates the baseline";
    let run = |tools: &Tools, command: String| {
        let result = call(tools, "bash", &[("command", Value::string(command))]);
        let code = result.get("exit_code").and_then(Value::as_usize);
        assert_eq!(code, Some(0), "{result:?}");
    };
    bench.prepare("out/ra.txt", b"rename\n");
    bench.prepare("out/d.txt", b"delete\n");
    let samples = recorder.count(30);
    let cases: [Case; 5] = [
        ("read-only command", |_| {
            "cat src/d000/f00000.rs > /dev/null".into()
        }),
        ("one changed file", |sample| {
            format!("printf 'v{sample}' > out/one.txt")
        }),
        ("100 changed files", |sample| {
            format!(
                "mkdir -p out/m && for i in $(seq 1 100); do printf 'v{sample}' > out/m/$i.txt; done"
            )
        }),
        ("rename a file", |sample| match sample % 2 {
            0 => "mv out/ra.txt out/rb.txt".into(),
            _ => "mv out/rb.txt out/ra.txt".into(),
        }),
        ("delete or create a file", |sample| match sample % 2 {
            0 => "rm out/d.txt".into(),
            _ => "printf 'delete\\n' > out/d.txt".into(),
        }),
    ];
    for (case, command) in cases {
        bench.measure(
            recorder,
            [workflow, case],
            note,
            samples,
            Duration::ZERO,
            |tools, _, sample| {
                run(tools, command(sample));
            },
        );
    }
    bench.captured("out/one.txt");
    bench.captured("out/m/100.txt");
    let samples = recorder.count(10);
    bench.measure(recorder, [workflow, "burst of 1,000 changed files"], note, samples, Duration::ZERO, |tools, _, sample| {
        run(tools, format!("mkdir -p out/b && for i in $(seq 1 1000); do printf 'v{sample}' > out/b/$i.txt; done"));
    });
    bench.captured("out/b/1000.txt");
}

/// A background command ends; the delay from its last write to the capture.
fn background(recorder: &mut Recorder, bench: &Bench) {
    let tools = &bench.tools[1];
    let tree = &bench.trees[1];
    let mut delays = Vec::new();
    for sample in 0..recorder.count(20) {
        let expected = format!("v{sample}");
        let command = format!(
            "sleep 0.3; mkdir -p out/bg; for i in $(seq 1 20); do printf '{expected}' > out/bg/$i.txt; done"
        );
        let fields = [
            ("command", Value::string(command)),
            ("wait_ms", Value::number(0)),
        ];
        call(tools, "bash", &fields);
        let last = tree.path("out/bg/20.txt");
        let started = Instant::now();
        while fs::read(&last).ok().as_deref() != Some(expected.as_bytes()) {
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "the command did not run"
            );
            std::thread::sleep(Duration::from_micros(200));
        }
        let written = Instant::now();
        let digest = hash::digest(expected.as_bytes());
        loop {
            let captured = matches!(
                bench.workspace().store().index.current("out/bg/20.txt").map(|version| &version.state),
                Some(State::File { digest: found, .. }) if *found == digest
            );
            if captured {
                break;
            }
            assert!(
                written.elapsed() < Duration::from_secs(30),
                "the background write was not captured"
            );
            std::thread::sleep(Duration::from_micros(200));
        }
        delays.push(written.elapsed().as_secs_f64() * 1000.0);
    }
    recorder.add(
        [
            "bash",
            "background command: last write until captured",
            "total",
        ],
        "persistent process (wait_ms 0) writing 20 files; polled every 0.2 ms",
        delays,
    );
}
