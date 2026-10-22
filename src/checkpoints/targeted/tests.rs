use super::*;
use crate::checkpoints::actions::{self, Caller};
use crate::checkpoints::restore::Expected;
use crate::checkpoints::tests::{Fixture, index, noise, paths, request};
use std::fs;

fn selective(fixture: &Fixture) -> Arc<Workspace> {
    let workspace = crate::checkpoints::workspace::open(
        fixture.home.path(),
        fixture.root.path(),
        crate::checkpoints::workspace::Options::offline(),
    )
    .unwrap();
    workspace
        .configure(Some(1 << 20), Some(256 << 10), "test")
        .unwrap();
    for number in 0..4 {
        fixture.write(&format!("data/{number}.bin"), &noise(number, 128 << 10));
    }
    // Simulate a previously interrupted, selective capture. A soft budget no
    // longer makes a healthy workspace selective merely because it is large.
    workspace
        .limit_scope("synthetic incomplete baseline")
        .unwrap();
    workspace
        .signals
        .update(|health| health.readiness = crate::checkpoints::workspace::Readiness::Ready);
    assert!(workspace.selective().is_some());
    workspace
}

fn call(workspace: &Arc<Workspace>, names: &[&str]) -> Value {
    actions::run(
        workspace,
        &Caller {
            origin: "model",
            user: false,
        },
        &Value::object([
            ("action", Value::string("capture")),
            (
                "paths",
                Value::Array(names.iter().copied().map(Value::string).collect()),
            ),
        ]),
    )
    .unwrap()
}

fn item<'a>(result: &'a Value, path: &str) -> &'a Value {
    result
        .get("paths")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item.get("path").and_then(Value::as_str) == Some(path))
        .unwrap()
}

#[test]
fn selected_preimages_survive_bash_like_changes_and_restore_exact_bytes() {
    let fixture = Fixture::new();
    let original = b"first\0binary\r\n";
    fixture.write("work.bin", original);
    let workspace = selective(&fixture);
    let before_capture = workspace
        .mark("turn", "before", "model", false, false)
        .unwrap();
    assert!(index(&workspace).at("work.bin", before_capture).is_none());
    let captured = call(&workspace, &["work.bin", "created.txt"]);
    assert_eq!(captured.get("saved").and_then(Value::as_usize), Some(2));
    let point =
        crate::checkpoints::index::parse_reference(captured.get("ref").unwrap().as_str().unwrap())
            .unwrap();
    assert_eq!(
        item(&captured, "created.txt")
            .get("state")
            .and_then(Value::as_str),
        Some("absent")
    );
    let command = workspace.command("model", "change selected paths");
    fixture.write("work.bin", b"second\0bytes\n");
    fixture.write("created.txt", b"new\n");
    command.finish();
    let selected = paths(&["work.bin", "created.txt"]);
    let report = workspace
        .restore(&request(
            &workspace,
            point,
            Some(&selected),
            Expected::Latest,
        ))
        .unwrap();
    assert!(
        report.failed.is_none() && report.conflicts.is_empty(),
        "{:?}",
        report.failed
    );
    assert_eq!(fixture.read("work.bin").unwrap(), original);
    assert!(!fixture.path("created.txt").exists());
    // The old turn knew neither path. Its unknown state never means delete.
    let old = workspace
        .restore(&request(&workspace, before_capture, None, Expected::Latest))
        .unwrap();
    assert!(old.failed.is_none());
    assert_eq!(fixture.read("work.bin").unwrap(), original);
}

#[test]
fn excluded_directory_and_oversized_paths_report_no_false_save() {
    let fixture = Fixture::new();
    fixture.write("work.txt", b"small");
    fixture.write("big.bin", &noise(57, 300 << 10));
    let workspace = selective(&fixture);
    let captured = call(&workspace, &["work.txt", "data", ".git/config", "big.bin"]);
    assert_eq!(captured.get("saved").and_then(Value::as_usize), Some(1));
    assert_eq!(captured.get("skipped").and_then(Value::as_usize), Some(2));
    assert_eq!(
        captured.get("unprotected").and_then(Value::as_usize),
        Some(1)
    );
    assert_eq!(
        item(&captured, "data")
            .get("status")
            .and_then(Value::as_str),
        Some("skipped")
    );
    assert_eq!(
        item(&captured, ".git/config")
            .get("status")
            .and_then(Value::as_str),
        Some("skipped")
    );
    assert_eq!(
        item(&captured, "big.bin")
            .get("status")
            .and_then(Value::as_str),
        Some("unprotected")
    );
    assert!(item(&captured, "big.bin").get("saved_at").is_none());
    assert!(index(&workspace).current("big.bin").is_none());
}

#[test]
fn an_unchanged_old_version_returns_a_retained_saved_reference_after_collection() {
    let fixture = Fixture::new();
    fixture.write("stable.txt", b"stable original\n");
    let workspace = fixture.open();
    workspace
        .configure(Some(1 << 20), Some(128 << 10), "test")
        .unwrap();
    for seed in 0..20 {
        fixture.write("churn.bin", &noise(seed, 80 << 10));
        workspace
            .mark("milestone", "churn", "test", false, false)
            .unwrap();
    }
    workspace.collect(true).unwrap().unwrap();
    let snapshot = index(&workspace);
    let original = snapshot.current("stable.txt").unwrap().seq;
    assert!(original < snapshot.horizon);
    assert!(!snapshot.retained(original));

    let captured = call(&workspace, &["stable.txt"]);
    let saved = crate::checkpoints::index::parse_reference(
        item(&captured, "stable.txt")
            .get("saved_at")
            .unwrap()
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert!(index(&workspace).retained(saved), "{captured:?}");
    fixture.write("stable.txt", b"later change\n");
    workspace
        .restore(&request(
            &workspace,
            saved,
            Some(&paths(&["stable.txt"])),
            Expected::Latest,
        ))
        .unwrap();
    assert_eq!(fixture.read("stable.txt").unwrap(), b"stable original\n");
}

#[test]
fn a_same_size_external_edit_during_capture_has_no_saved_outcome() {
    let fixture = Fixture::new();
    fixture.write("work.txt", b"aaaa");
    let workspace = selective(&fixture);
    let result = run_with(&workspace, "model", &paths(&["work.txt"]), |_| {
        fs::write(fixture.path("work.txt"), b"bbbb").unwrap();
    })
    .unwrap();
    assert_eq!(result.get("saved").and_then(Value::as_usize), Some(0));
    assert!(result.get("ref").is_none());
    assert_eq!(
        item(&result, "work.txt")
            .get("status")
            .and_then(Value::as_str),
        Some("unprotected")
    );
    assert!(item(&result, "work.txt").get("saved_at").is_none());
    assert_eq!(fixture.read("work.txt").unwrap(), b"bbbb");
}

#[test]
fn capture_then_bash_preserves_a_later_writer_until_its_version_is_accepted() {
    let fixture = Fixture::new();
    fixture.write("work.txt", b"original\n");
    let workspace = selective(&fixture);
    let captured = call(&workspace, &["work.txt"]);
    let target =
        crate::checkpoints::index::parse_reference(captured.get("ref").unwrap().as_str().unwrap())
            .unwrap();
    let command = workspace.command("model", "change work.txt");
    fixture.write("work.txt", b"our Bash change\n");
    command.finish();
    fixture.write("work.txt", b"colleague change\n");
    workspace.verify().unwrap();
    let selected = paths(&["work.txt"]);
    let conflict = workspace
        .restore(&request(&workspace, target, Some(&selected), Expected::Own))
        .unwrap();
    assert_eq!(conflict.conflicts.len(), 1);
    assert_eq!(fixture.read("work.txt").unwrap(), b"colleague change\n");
    let accepted = conflict.conflicts[0].current;
    let restored = workspace
        .restore(&request(
            &workspace,
            target,
            Some(&selected),
            Expected::At(accepted),
        ))
        .unwrap();
    assert!(restored.conflicts.is_empty() && restored.failed.is_none());
    assert_eq!(fixture.read("work.txt").unwrap(), b"original\n");
}

#[test]
fn offline_capture_command_saves_a_named_path_without_provider() {
    let fixture = Fixture::new();
    fixture.write("work.txt", b"before\n");
    fs::write(fixture.home.path().join("config.json"), "{invalid provider").unwrap();
    let output = crate::checkpoints::command::offline(
        fixture.home.path(),
        fixture.root.path(),
        &paths(&["capture", "work.txt"]),
    )
    .unwrap()
    .text();
    assert!(output.contains("captured 1 path"), "{output}");
    assert!(output.contains("saved at: cp-"), "{output}");
}

#[test]
fn a_full_active_generation_does_not_claim_a_saved_capture() {
    let fixture = Fixture::new();
    fixture.write("work.txt", b"before\n");
    let workspace = selective(&fixture);
    let generation = workspace.store().journal.generation;
    let used = crate::checkpoints::quota::measure(&workspace.archive, generation).unwrap();
    workspace.store().journal.quota_limit = 2 * used.generation;
    let captured = call(&workspace, &["work.txt"]);
    assert_eq!(captured.get("saved").and_then(Value::as_usize), Some(0));
    assert_eq!(
        captured.get("unprotected").and_then(Value::as_usize),
        Some(1)
    );
    assert!(captured.get("ref").is_none());
    let path = item(&captured, "work.txt");
    assert!(path.get("saved_at").is_none());
    assert!(
        path.get("reason")
            .and_then(Value::as_str)
            .unwrap()
            .contains("capacity"),
        "{path:?}"
    );
    assert!(index(&workspace).current("work.txt").is_none());
}

#[test]
#[ignore = "release measurement; run explicitly with --ignored"]
fn measure_explicit_capture_in_selective_workspace() {
    use std::time::Instant;

    let fixture = Fixture::new();
    let workspace = crate::checkpoints::workspace::open(
        fixture.home.path(),
        fixture.root.path(),
        crate::checkpoints::workspace::Options::offline(),
    )
    .unwrap();
    workspace
        .configure(Some(128 << 20), Some(2 << 20), "test")
        .unwrap();
    for number in 0..48 {
        fixture.write(&format!("data/{number}.bin"), &noise(number, 700 << 10));
    }
    let small = noise(41, 4 << 10);
    let large = noise(42, 1 << 20);
    fixture.write("work4k.bin", &small);
    fixture.write("work1m.bin", &large);
    let fresh_paths = |prefix: &str, count: usize, length: usize| -> Vec<String> {
        (0..count)
            .map(|number| {
                let path = format!("{prefix}/{number}.bin");
                fixture.write(&path, &noise(10_000 + number as u64, length));
                path
            })
            .collect()
    };
    let fresh_small = fresh_paths("first4k", 200, 4 << 10);
    let fresh_large = fresh_paths("first1m", 20, 1 << 20);
    workspace
        .limit_scope("synthetic incomplete baseline")
        .unwrap();
    workspace
        .signals
        .update(|health| health.readiness = crate::checkpoints::workspace::Readiness::Ready);
    assert!(workspace.selective().is_some());

    fn samples(workspace: &Arc<Workspace>, label: &str, names: &[String], first: bool) {
        let count = names.len();
        let mut micros = Vec::with_capacity(count);
        for path in names {
            if first {
                assert!(workspace.store().index.current(path).is_none());
            }
            let start = Instant::now();
            let result = call(workspace, &[path]);
            micros.push(start.elapsed().as_micros());
            assert_eq!(
                result.get("saved").and_then(Value::as_usize),
                Some(1),
                "{result:?}"
            );
            let live = fs::read(workspace.root.join(path)).unwrap();
            let store = workspace.store();
            let State::File { object, .. } = store.index.current(path).unwrap().state else {
                panic!("captured path is not a file: {path}");
            };
            assert_eq!(store.bytes(object).unwrap()[..], live[..]);
            assert!(
                result.get("ref").and_then(Value::as_str).is_some(),
                "{result:?}"
            );
        }
        let mut sorted = micros.clone();
        sorted.sort_unstable();
        let median = sorted[count / 2];
        let p95 = (count >= 100).then(|| sorted[(count * 95).div_ceil(100) - 1]);
        println!("targeted_capture.{label}.samples_us={micros:?}");
        println!("targeted_capture.{label}.median_us={median}");
        if let Some(p95) = p95 {
            println!("targeted_capture.{label}.p95_us={p95}");
        }
    }

    call(&workspace, &["work4k.bin", "work1m.bin"]);
    samples(
        &workspace,
        "repeat4k",
        &vec!["work4k.bin".into(); 200],
        false,
    );
    samples(
        &workspace,
        "repeat1m",
        &vec!["work1m.bin".into(); 20],
        false,
    );
    samples(&workspace, "first4k", &fresh_small, true);
    samples(&workspace, "first1m", &fresh_large, true);

    let saved = call(&workspace, &["work4k.bin", "work1m.bin"]);
    assert_eq!(saved.get("saved").and_then(Value::as_usize), Some(2));
    let point =
        crate::checkpoints::index::parse_reference(saved.get("ref").unwrap().as_str().unwrap())
            .unwrap();
    let command = workspace.command("model", "Bash modifies the selected paths");
    let bash = std::process::Command::new(crate::tools::find_bash().unwrap())
        .arg("-c")
        .arg("printf changed > work4k.bin; printf changed > work1m.bin")
        .current_dir(fixture.root.path())
        .status()
        .unwrap();
    assert!(bash.success());
    let report = command.finish();
    assert!(report.error.is_none(), "{:?}", report.error);
    let selected = paths(&["work4k.bin", "work1m.bin"]);
    let restored = workspace
        .restore(&request(
            &workspace,
            point,
            Some(&selected),
            Expected::Latest,
        ))
        .unwrap();
    assert!(restored.failed.is_none() && restored.conflicts.is_empty());
    assert_eq!(fixture.read("work4k.bin").unwrap(), small);
    assert_eq!(fixture.read("work1m.bin").unwrap(), large);
}
