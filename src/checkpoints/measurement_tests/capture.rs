//! Initial capture, reopen, points without changes and notification timing.
use super::*;
use crate::checkpoints::record::State;

pub fn run(recorder: &mut Recorder) {
    // The explicit full scan is a legacy comparison workload; ordinary
    // on-demand startup is measured separately.
    #[cfg(windows)]
    helper(recorder);
    for (files, samples) in [(1_000, 10), (10_000, 5)] {
        let samples = recorder.count(samples);
        initial_and_reopen(recorder, files, samples);
    }
    for (files, samples) in [(1_000, 200), (10_000, 50)] {
        let samples = recorder.count(samples);
        points(recorder, files, samples);
    }
}

/// Drops a workspace and waits until no thread still uses it.
pub(super) fn close(workspace: Arc<Workspace>) {
    let weak = Arc::downgrade(&workspace);
    drop(workspace);
    let started = Instant::now();
    while weak.strong_count() > 0 {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "the workspace stayed open"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn initial_and_reopen(recorder: &mut Recorder, files: usize, samples: usize) {
    let workflow = "capture";
    let case = |what: &str| format!("{files} files: {what}");
    let mut series: [Vec<f64>; 6] = Default::default();
    for sample in 0..samples {
        let tree = Tree::new(files, 1 + sample as u64);
        let source: u64 = tree
            .files
            .iter()
            .map(|file| fs::metadata(tree.path(file)).unwrap().len())
            .sum();
        let (workspace, initial) = timed(|| tree.open());
        series[0].push(initial);
        if sample == 0 {
            let archive = workspace.archive.clone();
            recorder.metric(
                workflow,
                &case("initial capture"),
                "workspace bytes",
                source as f64,
                "bytes",
            );
            recorder.metric(
                workflow,
                &case("initial capture"),
                "archive bytes",
                archive_bytes(&archive) as f64,
                "bytes",
            );
            recorder.counters(
                [workflow, &case("initial capture")],
                Counters::default(),
                counters(&workspace),
                1,
            );
        }
        // Full rereads find edits that preserved size and modification time.
        let (summary, verify) = timed(|| workspace.verify().unwrap());
        assert!(summary.changed.is_empty());
        series[1].push(verify);
        close(workspace);

        let (workspace, reopen) = timed(|| tree.open());
        series[2].push(reopen);
        close(workspace);
        let (workspace, offline) = timed(|| tree.open_with(Options::offline()));
        series[3].push(offline);
        close(workspace);

        // Changes made while no process watched: 1% edited, 0.5% deleted, 0.5% new.
        let edited = &tree.files[..files / 100];
        let deleted = &tree.files[files / 100..files / 100 + files / 200];
        for (index, relative) in edited.iter().enumerate() {
            tree.write(relative, &text(10_000 + index as u64, 2_000));
        }
        for relative in deleted {
            fs::remove_file(tree.path(relative)).unwrap();
        }
        let created: Vec<String> = (0..files / 200)
            .map(|index| format!("new/n{index:05}.rs"))
            .collect();
        for (index, relative) in created.iter().enumerate() {
            tree.write(relative, &text(20_000 + index as u64, 1_500));
        }
        let (workspace, mutated) = timed(|| tree.open());
        series[4].push(mutated);
        // Verified outside the timer.
        let store = workspace.store();
        for relative in edited.iter().chain(&created) {
            let Some(version) = store.index.current(relative) else {
                panic!("{relative} was not captured");
            };
            let State::File { length, .. } = version.state else {
                panic!("{relative} is not a file");
            };
            assert_eq!(length, fs::metadata(tree.path(relative)).unwrap().len());
        }
        for relative in deleted {
            assert_eq!(
                store.index.current(relative).map(|version| &version.state),
                Some(&State::Absent)
            );
        }
        drop(store);
        close(workspace);
        let (workspace, offline) = timed(|| tree.open_with(Options::offline()));
        series[5].push(offline);
        close(workspace);
    }
    let [initial, verify, reopen, offline, mutated, offline_after] = series;
    let note =
        "legacy explicit full scan; warm OS cache; time until all eligible paths are recorded";
    recorder.add(
        [workflow, &case("initial capture (empty archive)"), "total"],
        note,
        initial,
    );
    recorder.add(
        [workflow, &case("full reread (verify)"), "total"],
        "hashes every file",
        verify,
    );
    recorder.add(
        [workflow, &case("reopen, unchanged"), "total"],
        note,
        reopen,
    );
    recorder.add(
        [workflow, &case("reopen, unchanged, offline CLI"), "total"],
        "Options::offline, no helper",
        offline,
    );
    recorder.add(
        [
            workflow,
            &case("reopen after offline changes (2%)"),
            "total",
        ],
        note,
        mutated,
    );
    recorder.add(
        [
            workflow,
            &case("reopen after reconcile, offline CLI"),
            "total",
        ],
        "Options::offline, no helper",
        offline_after,
    );
}

/// Points with nothing to capture: the freshness work, a reused point and a
/// newly recorded one.
fn points(recorder: &mut Recorder, files: usize, samples: usize) {
    let workflow = "points";
    let tree = Tree::new(files, 7);
    // Without the helper, Windows polls as Unix always does.
    let polling = Options {
        watch: false,
        ..Options::interactive()
    };
    let variants = [
        Some(("product options", Options::interactive())),
        cfg!(windows).then_some(("polling, no helper", polling)),
    ];
    for (variant, options) in variants.into_iter().flatten() {
        let workspace = tree.open_with(options);
        let watching = format!("{:?}", workspace.watching());
        let note = format!("{files} files; notifications {watching}");
        workspace
            .mark("turn", "first", "measure", false, false)
            .unwrap();
        let before = counters(&workspace);
        let freshness: Vec<f64> = (0..samples)
            .map(|_| timed(|| workspace.catch_up("measure").unwrap()).1)
            .collect();
        recorder.counters(
            [
                workflow,
                &format!("{files} files: freshness only ({variant})"),
            ],
            before,
            counters(&workspace),
            samples,
        );
        let reused: Vec<f64> = (0..samples)
            .map(|_| {
                timed(|| {
                    workspace
                        .mark("turn", "again", "measure", false, true)
                        .unwrap()
                })
                .1
            })
            .collect();
        let recorded: Vec<f64> = (0..samples)
            .map(|_| {
                timed(|| {
                    workspace
                        .mark("turn", "new", "measure", false, false)
                        .unwrap()
                })
                .1
            })
            .collect();
        recorder.add(
            [workflow, &format!("{files} files: freshness only"), variant],
            &note,
            freshness,
        );
        recorder.add(
            [
                workflow,
                &format!("{files} files: reused point (turn boundary)"),
                variant,
            ],
            &note,
            reused,
        );
        recorder.add(
            [
                workflow,
                &format!("{files} files: new point record"),
                variant,
            ],
            &note,
            recorded,
        );
        #[cfg(windows)]
        if workspace.watching() == Watching::Active {
            rescan(recorder, &workspace, files);
        }
        close(workspace);
    }
}

/// A notification loss or overflow forces a verified rescan.
#[cfg(windows)]
fn rescan(recorder: &mut Recorder, workspace: &Workspace, files: usize) {
    let samples = recorder.count(5);
    let times = (0..samples)
        .map(|_| {
            timed(|| {
                workspace
                    .signals
                    .update(|health| health.rescan = Some("measurement".into()));
                workspace.flush("measure").unwrap();
            })
            .1
        })
        .collect();
    recorder.add(
        [
            "notifications",
            &format!("{files} files: forced rescan"),
            "total",
        ],
        "as after a lost helper or overflow; rereads every file",
        times,
    );
}

/// Helper start, cold in a fresh process and warm afterwards, and the delay
/// from an external write to its capture.
#[cfg(windows)]
fn helper(recorder: &mut Recorder) {
    let workflow = "notifications";
    let tree = Tree::new(100, 3);
    let start = |tree: &Tree| {
        let workspace =
            workspace::open(tree.home.path(), tree.root.path(), Options::interactive()).unwrap();
        let started = Instant::now();
        workspace.initialize().unwrap();
        let deadline = Duration::from_secs(120);
        while workspace.watching() != Watching::Active {
            assert!(started.elapsed() < deadline, "{:?}", workspace.watching());
            std::thread::sleep(Duration::from_micros(200));
        }
        (workspace, started.elapsed().as_secs_f64() * 1000.0)
    };
    let (workspace, cold) = start(&tree);
    recorder.add(
        [workflow, "helper start, first in process", "total"],
        "explicit full scan starts the notification helper; first helper may compile",
        vec![cold],
    );
    ready(&workspace);
    close(workspace);
    let mut warm = Vec::new();
    for _ in 0..recorder.count(10) {
        let (workspace, time) = start(&tree);
        warm.push(time);
        ready(&workspace);
        close(workspace);
    }
    recorder.add(
        [workflow, "helper start, compiled", "total"],
        "explicit full scan, including already compiled helper readiness",
        warm,
    );

    let workspace = tree.open();
    let mut delays = Vec::new();
    for sample in 0..recorder.count(100) {
        let relative = "src/d000/f00000.rs";
        let length = 1_000 + sample as u64;
        tree.write(relative, &text(sample as u64, length as usize));
        let started = Instant::now();
        loop {
            let captured = matches!(
                workspace.store().index.current(relative).map(|version| &version.state),
                Some(State::File { length: found, .. }) if *found == length
            );
            if captured {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "the write was not captured"
            );
            std::thread::sleep(Duration::from_micros(200));
        }
        delays.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    recorder.add(
        [workflow, "external write until captured", "total"],
        "includes the 20 ms debounce; polled every 0.2 ms",
        delays,
    );
    close(workspace);
}
