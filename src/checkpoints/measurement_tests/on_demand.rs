//! Product startup with no implicit workspace scan, followed by a declared
//! one-file Bash mutation and exact restore in a large untouched scope.
use super::capture::close;
use super::editing::call;
use super::*;
use crate::checkpoints::hash;
use crate::checkpoints::session::Session;
use crate::json::Value;
use crate::tools::Tools;

const BULK_FILES: usize = 33;
const BULK_BYTES: usize = 8 << 20;
const MANY_FILES: usize = 50_010;

pub fn run(recorder: &mut Recorder) {
    for (files, samples) in [(1_000, 10), (10_000, 5)] {
        let tree = Tree::new(files, files as u64);
        open_without_capture(
            recorder,
            tree.root.path(),
            &format!("{files} source files"),
            samples,
        );
    }

    let many = Directory::new();
    let directory = many.path().join("many");
    fs::create_dir_all(&directory).unwrap();
    let (_, setup) = timed(|| {
        for number in 0..MANY_FILES {
            fs::File::create(directory.join(format!("f{number:05}.txt"))).unwrap();
        }
    });
    recorder.add(
        ["on-demand", "50,010 empty files fixture creation", "setup"],
        "outside startup timer",
        vec![setup],
    );
    open_without_capture(recorder, many.path(), "50,010 empty files", 3);

    let bulk = Directory::new();
    fs::create_dir_all(bulk.path().join("bulk")).unwrap();
    fs::create_dir_all(bulk.path().join("work")).unwrap();
    let (sentinel, setup) = timed(|| {
        let mut sentinel = None;
        for number in 0..BULK_FILES {
            let bytes = binary(number as u64 + 100, BULK_BYTES);
            if number == 0 {
                sentinel = Some(hash::digest(&bytes));
            }
            fs::write(bulk.path().join(format!("bulk/f{number:03}.bin")), bytes).unwrap();
        }
        sentinel.unwrap()
    });
    recorder.add(
        ["on-demand", "264 MiB content fixture creation", "setup"],
        "outside startup timer; 33 distinct 8 MiB files",
        vec![setup],
    );
    open_without_capture(recorder, bulk.path(), "264 MiB untouched payload", 3);
    targeted_command_and_recovery(recorder, &bulk, sentinel);
}

fn open_without_capture(recorder: &mut Recorder, root: &Path, case: &str, normal: usize) {
    let mut times = Vec::new();
    for _ in 0..recorder.count(normal) {
        let home = Directory::new();
        let (workspace, ms) = timed(|| {
            let workspace = workspace::open(home.path(), root, Options::interactive()).unwrap();
            ready(&workspace);
            workspace
        });
        let store = workspace.store();
        assert!(store.index.paths.is_empty(), "startup scanned {case}");
        assert_eq!(store.counters.bytes_read, 0, "startup read {case} contents");
        if times.is_empty() {
            recorder.metric(
                "on-demand",
                case,
                "recorded paths before first tool",
                0.0,
                "paths",
            );
            recorder.metric(
                "on-demand",
                case,
                "file payload bytes read before first tool",
                0.0,
                "bytes",
            );
            recorder.metric(
                "on-demand",
                case,
                "archive bytes before first tool",
                archive_bytes(&workspace.archive) as f64,
                "bytes",
            );
        }
        drop(store);
        times.push(ms);
        close(workspace);
    }
    recorder.add(
        [
            "on-demand",
            &format!("{case}: open before first tool"),
            "total",
        ],
        "fresh archive per sample; no full scan; warm OS cache after fixture creation",
        times,
    );
}

fn targeted_command_and_recovery(
    recorder: &mut Recorder,
    bulk: &Directory,
    sentinel: hash::Digest,
) {
    let home = Directory::new();
    let target = bulk.path().join("work/one.txt");
    fs::write(&target, b"original\n").unwrap();
    let mut tools = Tools::new(bulk.path()).unwrap();
    tools.configure_output(home.path().join("logs"), crate::redact::Redactor::empty());
    let session = Session::open(home.path(), tools.root());
    session.begin("measure", None);
    ready(session.workspace().unwrap());
    let workspace = Arc::clone(session.workspace().unwrap());
    assert!(workspace.store().index.paths.is_empty());
    tools.configure_checkpoints(Arc::clone(&session));

    let samples = recorder.count(30);
    let mut times = Vec::new();
    let mut before = String::new();
    for sample in 0..samples {
        let command = format!("printf 'v{sample}' > work/one.txt");
        let fields = [
            ("command", Value::string(command)),
            ("protect", Value::Array(vec![Value::string("work/one.txt")])),
        ];
        let (result, ms) = timed(|| call(&tools, "bash", &fields));
        assert_eq!(result.get("exit_code").and_then(Value::as_usize), Some(0));
        let checkpoint = result.get("checkpoint").unwrap();
        assert_eq!(
            checkpoint.get("status").and_then(Value::as_str),
            Some("saved")
        );
        before = checkpoint
            .get("before")
            .and_then(Value::as_str)
            .unwrap()
            .into();
        assert_eq!(fs::read(&target).unwrap(), format!("v{sample}").as_bytes());
        times.push(ms);
    }
    recorder.add(
        ["on-demand", "declared Bash changing one file in 264 MiB scope", "total"],
        "foreground Bash; exact protect path; new content per call; untouched bulk files remain uncaptured",
        times,
    );
    let read = counters(&workspace).bytes_read;
    recorder.metric(
        "on-demand",
        "declared Bash changing one file in 264 MiB scope",
        "file payload bytes read",
        read as f64,
        "bytes",
    );
    assert!(read < BULK_BYTES as u64, "unrelated bulk payload was read");
    assert!(
        workspace
            .store()
            .index
            .paths
            .keys()
            .all(|path| !path.starts_with("bulk/"))
    );
    assert_eq!(
        hash::digest(&fs::read(bulk.path().join("bulk/f000.bin")).unwrap()),
        sentinel,
    );

    let paths = Value::Array(vec![Value::string("work/one.txt")]);
    let (restored, restore_ms) = timed(|| {
        call(
            &tools,
            "checkpoint",
            &[
                ("action", Value::string("restore")),
                ("ref", Value::string(&before)),
                ("paths", paths),
                ("expected", Value::string("latest")),
            ],
        )
    });
    assert!(restored.get("conflicts").is_none(), "{restored:?}");
    assert_eq!(
        fs::read(&target).unwrap(),
        format!("v{}", samples - 2).as_bytes()
    );
    recorder.add(
        [
            "on-demand",
            "restore the last declared Bash preimage",
            "total",
        ],
        "exact previous bytes verified outside timer",
        vec![restore_ms],
    );

    background_completion(recorder, &tools, &workspace, bulk);
    drop(tools);
    drop(session);
    close(workspace);
}

fn background_completion(
    recorder: &mut Recorder,
    tools: &Tools,
    workspace: &Arc<Workspace>,
    bulk: &Directory,
) {
    let mut times = Vec::new();
    for sample in 0..recorder.count(20) {
        let expected = format!("done{sample}");
        let command = format!("sleep 0.3; printf '{expected}' > work/bg.txt");
        let started = Instant::now();
        let launched = call(
            tools,
            "bash",
            &[
                ("command", Value::string(command)),
                ("protect", Value::Array(vec![Value::string("work/bg.txt")])),
                ("wait_ms", Value::number(0)),
            ],
        );
        let id = launched.get("process_id").and_then(Value::as_str).unwrap();
        loop {
            let report = call(
                tools,
                "process",
                &[
                    ("action", Value::string("poll")),
                    ("process_id", Value::string(id)),
                    ("wait_ms", Value::number(1000)),
                ],
            );
            if report.get("checkpoint_finalized") == Some(&Value::Bool(true)) {
                assert_eq!(report.get("status").and_then(Value::as_str), Some("exited"));
                assert_eq!(report.get("exit_code").and_then(Value::as_usize), Some(0));
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "background Bash did not finalize"
            );
        }
        let bytes = fs::read(bulk.path().join("work/bg.txt")).unwrap();
        assert_eq!(bytes, expected.as_bytes());
        let digest = hash::digest(&bytes);
        let store = workspace.store();
        let state = store.index.current_state("work/bg.txt");
        assert!(
            matches!(state, crate::checkpoints::record::State::File { digest: found, .. } if found == digest)
        );
        times.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    recorder.add(
        ["on-demand", "background Bash until checkpoint finalized", "total"],
        "wait_ms 0; 300 ms command delay; process poll returns only after exact declared path is captured",
        times,
    );
}
