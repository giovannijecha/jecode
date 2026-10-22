//! Opt-in measurements of complete acquisition and exact recovery beyond the
//! former discovery limits, including an individually large streamed file.
use super::*;
use crate::checkpoints::hash;
use crate::checkpoints::mutation::Protection;
use crate::checkpoints::restore::{Expected, Request};
use crate::test_support::Directory;

const BULK_FILES: usize = 33;
const BULK_BYTES: usize = 8 << 20;
const MANY_FILES: usize = 50_010;
const INITIAL_SAMPLES: usize = 3;

pub fn run(recorder: &mut Recorder) {
    content_limit(recorder);
    path_limit(recorder);
}

fn content_limit(recorder: &mut Recorder) {
    let root = Directory::new();
    fs::create_dir_all(root.path().join("bulk")).unwrap();
    fs::create_dir_all(root.path().join("work")).unwrap();
    let (sentinel, setup) = timed(|| {
        let mut sentinel = None;
        for number in 0..BULK_FILES {
            let bytes = binary(number as u64 + 100, BULK_BYTES);
            if number == 0 {
                sentinel = Some(hash::digest(&bytes));
            }
            fs::write(root.path().join(format!("bulk/f{number:03}.bin")), bytes).unwrap();
        }
        sentinel.unwrap()
    });
    recorder.add(
        ["large", "264 MiB content fixture creation", "setup"],
        "outside baseline and point timers; 33 distinct 8 MiB binary files",
        vec![setup],
    );

    let mut initial = Vec::new();
    for sample in 0..INITIAL_SAMPLES {
        let home = Directory::new();
        let (workspace, elapsed) = timed(|| {
            let workspace =
                workspace::open(home.path(), root.path(), Options::interactive()).unwrap();
            workspace.initialize().unwrap();
            ready(&workspace);
            workspace
        });
        initial.push(elapsed);
        if sample + 1 == INITIAL_SAMPLES {
            let protected = workspace.store().index.paths.len();
            assert!(
                protected >= BULK_FILES,
                "the complete large scope was not captured"
            );
            assert!(workspace.store().index.paths.values().all(|versions| {
                versions
                    .last()
                    .is_none_or(|version| version.state.recoverable())
            }));
            recorder.metric(
                "large",
                "264 MiB initial capture",
                "protected paths",
                protected as f64,
                "paths",
            );
            recorder.metric(
                "large",
                "264 MiB initial capture",
                "whole scope coverage",
                f64::from(workspace.store().index.coverage_complete() as u8),
                "boolean",
            );
            recorder.metric(
                "large",
                "264 MiB initial capture",
                "file bytes read",
                counters(&workspace).bytes_read as f64,
                "bytes",
            );
            let mut sampled_peak = archive_bytes(&workspace.archive);
            recorder.metric(
                "large",
                "264 MiB initial capture",
                "archive bytes",
                sampled_peak as f64,
                "bytes",
            );
            protected_points_and_restore(recorder, &workspace, &root, sentinel, &mut sampled_peak);
        }
        capture::close(workspace);
    }
    recorder.add(
        ["large", "264 MiB initial capture", "total"],
        "legacy explicit full scan; warm OS cache after fixture creation; three fresh archives",
        initial,
    );
}

fn protected_points_and_restore(
    recorder: &mut Recorder,
    workspace: &Workspace,
    root: &Directory,
    sentinel: hash::Digest,
    sampled_peak: &mut u64,
) {
    let first = root.path().join("work/a.txt");
    let second = root.path().join("work/b.txt");
    let before = [
        (first.as_path(), b"first before\n".as_slice()),
        (second.as_path(), b"second before\n".as_slice()),
    ];
    let after = [
        (first.as_path(), b"first after\n".as_slice()),
        (second.as_path(), b"second after\n".as_slice()),
    ];
    for (path, bytes) in before {
        save(workspace, path, bytes);
    }
    let target = workspace
        .mark("turn", "before", "measure", true, false)
        .unwrap();
    *sampled_peak = (*sampled_peak).max(archive_bytes(&workspace.archive));

    let times: Vec<f64> = (0..recorder.count(200))
        .map(|_| {
            timed(|| {
                workspace
                    .mark("turn", "reuse", "measure", false, true)
                    .unwrap()
            })
            .1
        })
        .collect();
    recorder.add(
        ["large", "264 MiB workspace, reused point", "total"],
        "one protected work file pair; each result includes durable point work",
        times,
    );
    for (path, bytes) in after {
        save(workspace, path, bytes);
    }
    *sampled_peak = (*sampled_peak).max(archive_bytes(&workspace.archive));

    let request = Request {
        target,
        paths: None,
        exact: false,
        expected: Expected::Latest,
        preview: false,
        origin: "measure",
        excluded: &workspace.excluded,
    };
    let (report, restore_ms) = timed(|| workspace.restore(&request).unwrap());
    assert!(report.conflicts.is_empty(), "full restore conflicted");
    assert!(report.failed.is_none(), "full restore failed");
    assert_eq!(fs::read(&first).unwrap(), b"first before\n");
    assert_eq!(fs::read(&second).unwrap(), b"second before\n");
    assert_eq!(
        hash::digest(&fs::read(root.path().join("bulk/f000.bin")).unwrap()),
        sentinel,
        "the untouched eligible sentinel changed or disappeared"
    );
    recorder.add(
        ["large", "full restore, two changed work files", "total"],
        "all restored bytes and one untouched eligible 8 MiB sentinel verified",
        vec![restore_ms],
    );
    *sampled_peak = (*sampled_peak).max(archive_bytes(&workspace.archive));
    std::thread::sleep(Duration::from_millis(250));
    let final_bytes = archive_bytes(&workspace.archive);
    *sampled_peak = (*sampled_peak).max(final_bytes);
    recorder.metric(
        "large",
        "264 MiB point and restore",
        "sampled archive peak",
        *sampled_peak as f64,
        "bytes",
    );
    recorder.metric(
        "large",
        "264 MiB point and restore",
        "archive bytes after 250 ms",
        final_bytes as f64,
        "bytes",
    );
}

fn save(workspace: &Workspace, path: &Path, bytes: &[u8]) {
    let mut mutation = workspace.mutation("measure", "write", path);
    mutation.prepare(bytes);
    fs::write(path, bytes).unwrap();
    assert!(
        matches!(mutation.finish(true), Protection::Protected { .. }),
        "the direct tool mutation was not protected"
    );
}

fn path_limit(recorder: &mut Recorder) {
    let root = Directory::new();
    let directory = root.path().join("many");
    fs::create_dir_all(&directory).unwrap();
    let (_, setup) = timed(|| {
        for number in 0..MANY_FILES {
            fs::File::create(directory.join(format!("f{number:05}.txt"))).unwrap();
        }
    });
    recorder.add(
        ["large", "50,010 empty files fixture creation", "setup"],
        "outside baseline timer; one directory with 50,010 distinct files",
        vec![setup],
    );
    let mut initial = Vec::new();
    for sample in 0..INITIAL_SAMPLES {
        let home = Directory::new();
        let (workspace, elapsed) = timed(|| {
            let workspace =
                workspace::open(home.path(), root.path(), Options::interactive()).unwrap();
            workspace.initialize().unwrap();
            ready(&workspace);
            workspace
        });
        initial.push(elapsed);
        if sample + 1 == INITIAL_SAMPLES {
            assert!(
                workspace.store().index.paths.len() >= MANY_FILES,
                "discovery silently truncated the scope"
            );
            recorder.metric(
                "large",
                "50,010 files initial capture",
                "protected paths",
                workspace.store().index.paths.len() as f64,
                "paths",
            );
            recorder.metric(
                "large",
                "50,010 files initial capture",
                "whole scope coverage",
                f64::from(workspace.store().index.coverage_complete() as u8),
                "boolean",
            );
            recorder.metric(
                "large",
                "50,010 files initial capture",
                "archive bytes",
                archive_bytes(&workspace.archive) as f64,
                "bytes",
            );
        }
        capture::close(workspace);
    }
    recorder.add(
        ["large", "50,010 files initial capture", "total"],
        "legacy explicit full scan; warm OS cache after fixture creation; three fresh archives",
        initial,
    );
}
