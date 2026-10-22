//! The archive's storage lifecycle: bytes per revision, collection under a
//! budget and the restart after compaction.
use super::capture::close;
use super::*;
use crate::checkpoints::hash;
use crate::checkpoints::record::State;

/// A named way to produce each revision's content.
type Revision<'a> = (&'static str, Box<dyn Fn(usize) -> Vec<u8> + 'a>);

/// Bytes per revision by storage kind, collection under a budget and the
/// restart after compaction.
pub fn run(recorder: &mut Recorder) {
    let workflow = "storage";
    let tree = Tree::new(100, 40);
    // Opened as `jecode checkpoints gc` opens it: a background worker would
    // start its own collection once the budget drops and race the timed one.
    let workspace = tree.open_with(Options::offline());
    let path = tree.path("s/data.rs");
    let save = |bytes: &[u8]| {
        let mut mutation = workspace.mutation("measure", "write", &path);
        mutation.prepare(bytes);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        mutation.finish(true);
    };
    let base = text(41, 40 * 1024);
    save(&base);
    let revisions = recorder.count(50);
    let kinds: [Revision; 3] = [
        (
            "small edit of 40 KiB text",
            Box::new(|revision| {
                let mut bytes = base.clone();
                bytes.extend_from_slice(format!("// small {revision}\n").as_bytes());
                bytes
            }),
        ),
        (
            "rewrite of 40 KiB text",
            Box::new(|revision| text(100 + revision as u64, 40 * 1024)),
        ),
        (
            "content written before",
            Box::new(|revision| {
                if revision % 2 == 0 {
                    base.clone()
                } else {
                    text(100, 40 * 1024)
                }
            }),
        ),
    ];
    for (case, content) in kinds {
        let before = counters(&workspace);
        let size = archive_bytes(&workspace.archive);
        let times = (0..revisions)
            .map(|revision| {
                std::thread::sleep(super::editing::pause(base.len()));
                timed(|| save(&content(revision))).1
            })
            .collect();
        let growth = archive_bytes(&workspace.archive).saturating_sub(size);
        recorder.counters([workflow, case], before, counters(&workspace), revisions);
        recorder.metric(
            workflow,
            case,
            "archive growth per revision",
            growth as f64 / revisions as f64,
            "bytes",
        );
        recorder.add(
            [workflow, case, "total"],
            "mutation API with durable prepare and finish; untimed pause before each save",
            times,
        );
    }
    let logical = archive_bytes(&workspace.archive);
    recorder.metric(
        workflow,
        "archive after revisions",
        "logical bytes",
        logical as f64,
        "bytes",
    );
    #[cfg(unix)]
    recorder.metric(
        workflow,
        "archive after revisions",
        "allocated bytes",
        allocated(&workspace.archive) as f64,
        "bytes",
    );

    // 64 MiB of binary history against a 16 MiB budget.
    let blobs = 64;
    for index in 0..blobs {
        save(&binary(200 + index, 1 << 20));
    }
    let before = archive_bytes(&workspace.archive);
    workspace
        .configure(Some(16 << 20), None, "measure")
        .unwrap();
    let (collection, collect) = timed(|| workspace.collect(false).unwrap());
    let collection = collection.expect("the archive exceeded its budget");
    recorder.add(
        [
            workflow,
            "collect 64 MiB of history to a 16 MiB budget",
            "total",
        ],
        "compaction into a new generation",
        vec![collect],
    );
    recorder.metric(
        workflow,
        "collection",
        "archive bytes before",
        before as f64,
        "bytes",
    );
    recorder.metric(
        workflow,
        "collection",
        "archive bytes after",
        archive_bytes(&workspace.archive) as f64,
        "bytes",
    );
    recorder.metric(
        workflow,
        "collection",
        "object bytes after",
        collection.after as f64,
        "bytes",
    );
    recorder.metric(
        workflow,
        "collection",
        "over budget",
        f64::from(u8::from(collection.over_budget)),
        "flag",
    );
    let samples = recorder.count(5);
    let forced = (0..samples)
        .map(|_| timed(|| workspace.collect(true).unwrap()).1)
        .collect();
    recorder.add(
        [workflow, "forced compaction, nothing to release", "total"],
        "rewrites the retained archive",
        forced,
    );
    let current = fs::read(&path).unwrap();
    close(workspace);
    let samples = recorder.count(10);
    let times = (0..samples)
        .map(|_| {
            let (workspace, time) = timed(|| tree.open_with(Options::offline()));
            let store = workspace.store();
            let state = store.index.current("s/data.rs").map(|version| version.state.clone());
            assert!(matches!(state, Some(State::File { digest, .. }) if digest == hash::digest(&current)));
            drop(store);
            close(workspace);
            time
        })
        .collect();
    recorder.add(
        [workflow, "reopen after compaction, offline CLI", "total"],
        "Options::offline",
        times,
    );
}

#[cfg(unix)]
fn allocated(directory: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    let mut total = 0;
    let mut pending = vec![directory.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).into_iter().flatten().flatten() {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                total += metadata.blocks() * 512;
            }
        }
    }
    total
}
