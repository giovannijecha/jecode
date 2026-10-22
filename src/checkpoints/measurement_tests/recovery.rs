//! Navigation, historical reads and restores.
use super::capture::close;
use super::editing::{Bench, call};
use super::*;
use crate::checkpoints::index::reference;
use crate::json::Value;
use crate::tools::Tools;

pub fn run(recorder: &mut Recorder) {
    let bench = Bench::new(1_000);
    let refs = history(&bench);
    navigation(recorder, &bench, &refs);
    restores(recorder, &bench);
    let archive = bench.workspace().archive.clone();
    let root = bench.trees[1].root.path().to_path_buf();
    let home = bench.trees[1].home.path().to_path_buf();
    reconstruction(recorder, bench, &refs, &home, &root, &archive);
}

fn checkpoint(tools: &Tools, fields: &[(&str, Value)]) -> Value {
    call(tools, "checkpoint", fields)
}

fn mark(bench: &Bench) -> String {
    let seq = bench
        .workspace()
        .mark("turn", "measure", "measure", false, false)
        .unwrap();
    reference(seq)
}

/// 100 distinct revisions of one 40 KiB file, each a small change.
fn history(bench: &Bench) -> Vec<String> {
    bench.directory("h");
    let tools = &bench.tools[1];
    let mut content = String::from_utf8(text(30, 40 * 1024)).unwrap();
    (0..100)
        .map(|revision| {
            content.push_str(&format!("// revision {revision}\n"));
            let fields = [
                ("path", Value::string("h/history.rs")),
                ("content", Value::string(&content)),
            ];
            let result = call(tools, "write", &fields);
            let after = result
                .get("checkpoint")
                .and_then(|value| value.get("after"));
            after.and_then(Value::as_str).unwrap().to_string()
        })
        .collect()
}

fn navigation(recorder: &mut Recorder, bench: &Bench, refs: &[String]) {
    let workflow = "navigation";
    let tools = &bench.tools[1];
    let note = "checkpoint tool call, 1,000-file workspace with 100 revisions of one file";
    let middle = Value::string(&refs[50]);
    let paths = Value::Array(vec![Value::string("h/history.rs")]);
    let cases: Vec<(&str, Vec<(&str, Value)>)> = vec![
        ("status", vec![("action", Value::string("status"))]),
        ("list", vec![("action", Value::string("list"))]),
        (
            "list a path's versions",
            vec![("action", Value::string("list")), ("paths", paths.clone())],
        ),
        (
            "inspect a point",
            vec![
                ("action", Value::string("inspect")),
                ("ref", middle.clone()),
            ],
        ),
        (
            "inspect a path at a point",
            vec![
                ("action", Value::string("inspect")),
                ("ref", middle),
                ("paths", paths),
            ],
        ),
    ];
    for (case, fields) in cases {
        let samples = recorder.count(100);
        let times = (0..samples)
            .map(|_| timed(|| checkpoint(tools, &fields)).1)
            .collect();
        recorder.add([workflow, case, "total"], note, times);
    }
    let samples = recorder.count(100);
    let reference = format!("checkpoint:{}:h/history.rs", refs[50]);
    let times = (0..samples)
        .map(|_| timed(|| call(tools, "read", &[("path", Value::string(&reference))])).1)
        .collect();
    recorder.add(
        [workflow, "read a historical version (cached)", "total"],
        "read tool; objects stay in the store cache after the first read",
        times,
    );
}

/// Historical reads in a fresh process-local store, so nothing is cached.
fn reconstruction(
    recorder: &mut Recorder,
    bench: Bench,
    refs: &[String],
    home: &Path,
    root: &Path,
    archive: &Path,
) {
    let (store_index, maximum) = {
        let store = bench.workspace().store();
        (
            store.index.objects.len(),
            crate::checkpoints::index::MAX_DEPTH,
        )
    };
    let Bench {
        trees,
        tools,
        session,
    } = bench;
    drop(tools);
    let workspace = Arc::clone(session.workspace().unwrap());
    drop(session);
    close(workspace);
    for back in [0, 9, 49, 99] {
        let target = &refs[refs.len() - 1 - back];
        let mut expected = String::from_utf8(text(30, 40 * 1024)).unwrap();
        for revision in 0..refs.len() - back {
            expected.push_str(&format!("// revision {revision}\n"));
        }
        let path = format!("checkpoint:{target}:h/history.rs");
        let samples = recorder.count(30);
        let times = (0..samples)
            .map(|_| {
                let workspace = workspace::open(home, root, Options::offline()).unwrap();
                workspace.initialize().unwrap();
                let (content, time) =
                    timed(|| crate::checkpoints::show::content(&workspace, &path).unwrap());
                match content {
                    crate::checkpoints::show::Content::Text(found) => assert_eq!(found, expected),
                    _ => panic!("historical text was not reconstructed"),
                }
                close(workspace);
                time
            })
            .collect();
        let case = format!("historical read, {back} revisions back, uncached");
        recorder.add(
            ["navigation", &case, "total"],
            &format!("fresh store per sample; delta chains are bounded at depth {maximum}"),
            times,
        );
    }
    recorder.metric(
        "storage",
        "100 revisions of a 40 KiB file",
        "archive objects",
        store_index as f64,
        "count",
    );
    recorder.metric(
        "storage",
        "100 revisions of a 40 KiB file",
        "archive bytes",
        archive_bytes(archive) as f64,
        "bytes",
    );
    drop(trees);
}

fn restores(recorder: &mut Recorder, bench: &Bench) {
    let workflow = "recovery";
    let tools = &bench.tools[1];
    let tree = &bench.trees[1];
    let note = "checkpoint tool restore; saves the current state, then installs";
    // Changes below come from outside this conversation, so restores replace
    // the latest state, as a user would; the conflict case keeps the default.
    let restore_with = |target: &str, paths: Option<&[String]>, latest: bool| {
        let mut fields = vec![("ref", Value::string(target))];
        if latest {
            fields.push(("expected", Value::string("latest")));
        }
        match paths {
            Some(paths) => {
                fields.push(("action", Value::string("restore")));
                fields.push((
                    "paths",
                    Value::Array(paths.iter().map(Value::string).collect()),
                ));
            }
            None => fields.push(("action", Value::string("restore_all"))),
        }
        checkpoint(tools, &fields)
    };
    let restore = |target: &str, paths: Option<&[String]>| restore_with(target, paths, true);
    let originals: Vec<(String, Vec<u8>)> = tree.files[..100]
        .iter()
        .map(|relative| (relative.clone(), fs::read(tree.path(relative)).unwrap()))
        .collect();
    let unchanged = |count: usize| {
        for (relative, bytes) in &originals[..count] {
            assert_eq!(&fs::read(tree.path(relative)).unwrap(), bytes, "{relative}");
        }
    };
    let change = |count: usize, sample: usize| {
        for (index, (relative, _)) in originals[..count].iter().enumerate() {
            tree.write(
                relative,
                &text(50_000 + (sample * 1000 + index) as u64, 1_000),
            );
        }
        mark(bench)
    };

    for (count, samples) in [(1, 50), (100, 10)] {
        let samples = recorder.count(samples);
        let paths: Vec<String> = originals[..count]
            .iter()
            .map(|(path, _)| path.clone())
            .collect();
        let (mut times, mut undo) = (Vec::new(), Vec::new());
        for sample in 0..samples {
            let target = mark(bench);
            change(count, sample);
            let (result, time) = timed(|| restore(&target, Some(&paths)));
            times.push(time);
            unchanged(count);
            let back = result
                .get("undo")
                .and_then(|undo| undo.get("ref"))
                .and_then(Value::as_str)
                .unwrap()
                .to_string();
            // As before the restore, notifications of the last writes have
            // settled; otherwise a point waits 30 ms for them to stop.
            std::thread::sleep(Duration::from_millis(100));
            undo.push(timed(|| restore(&back, Some(&paths))).1);
            for (index, (relative, _)) in originals[..count].iter().enumerate() {
                assert_eq!(
                    fs::read(tree.path(relative)).unwrap(),
                    text(50_000 + (sample * 1000 + index) as u64, 1_000),
                    "undo bytes differ for {relative}"
                );
            }
            // Undoing returns the changed bytes; restore the originals for the next sample.
            restore(&target, Some(&paths));
            unchanged(count);
        }
        recorder.add(
            [
                workflow,
                &format!("restore {count} changed file(s)"),
                "total",
            ],
            note,
            times,
        );
        recorder.add(
            [
                workflow,
                &format!("undo a restore of {count} file(s)"),
                "total",
            ],
            "restore of the undo ref after an untimed 100 ms pause",
            undo,
        );
    }

    // The whole 1,000-file workspace: 100 edits, 10 deletions and 10 new files.
    let samples = recorder.count(10);
    let mut times = Vec::new();
    for sample in 0..samples {
        let target = mark(bench);
        for (relative, _) in &originals[90..100] {
            fs::remove_file(tree.path(relative)).unwrap();
        }
        for index in 0..10 {
            tree.write(&format!("created/{sample}-{index}.txt"), b"new\n");
        }
        change(90, sample);
        let (result, time) = timed(|| restore(&target, None));
        assert!(result.get("conflicts").is_none(), "{result:?}");
        times.push(time);
        unchanged(100);
        assert!(
            !tree.path("created").exists()
                || fs::read_dir(tree.path("created")).unwrap().next().is_none()
        );
    }
    recorder.add(
        [
            workflow,
            "restore the whole workspace (120 changes)",
            "total",
        ],
        note,
        times,
    );

    // Recreate a deleted file, delete a created one and report a conflict.
    let samples = recorder.count(30);
    let (mut create, mut delete, mut conflict) = (Vec::new(), Vec::new(), Vec::new());
    let (relative, bytes) = &originals[0];
    let path = [relative.clone()];
    let fresh = ["fresh/new.txt".to_string()];
    for _ in 0..samples {
        let target = mark(bench);
        fs::remove_file(tree.path(relative)).unwrap();
        mark(bench);
        create.push(timed(|| restore(&target, Some(&path))).1);
        assert_eq!(&fs::read(tree.path(relative)).unwrap(), bytes);

        let target = mark(bench);
        tree.write(&fresh[0], b"created\n");
        mark(bench);
        delete.push(timed(|| restore(&target, Some(&fresh))).1);
        assert!(!tree.path(&fresh[0]).exists());

        // This conversation edits; someone else edits after it.
        let target = mark(bench);
        call(
            tools,
            "write",
            &[
                ("path", Value::string(relative)),
                ("content", Value::string("mine\n")),
            ],
        );
        tree.write(relative, b"theirs\n");
        mark(bench);
        let (result, time) = timed(|| restore_with(&target, Some(&path), false));
        assert!(result.get("conflicts").is_some(), "{result:?}");
        conflict.push(time);
        assert_eq!(fs::read(tree.path(relative)).unwrap(), b"theirs\n");
        tree.write(relative, bytes);
        mark(bench);
    }
    recorder.add([workflow, "recreate a deleted file", "total"], note, create);
    recorder.add(
        [workflow, "delete a file created after the point", "total"],
        note,
        delete,
    );
    recorder.add(
        [workflow, "restore stopped by a conflict", "total"],
        "no file changes; the conflict is reported",
        conflict,
    );
}
