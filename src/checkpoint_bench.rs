//! Opt-in matched tool workloads. No network or user workspace is touched.
use crate::checkpoints::session::Session;
use crate::json::Value;
use crate::test_support::Directory;
use crate::tools::Tools;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Instant;

fn call(tools: &Tools, name: &str, fields: Vec<(&str, Value)>) -> Value {
    let args = Value::Object(fields.into_iter().map(|(k, v)| (k.into(), v)).collect());
    let result = tools.execute(name, &args.encode());
    assert!(result.get("error").is_none(), "{name}: {result:?}");
    result
}

fn capture(tools: &Tools, paths: &[String]) -> String {
    let result = call(
        tools,
        "bash",
        vec![
            ("command", Value::string(":")),
            (
                "protect",
                Value::Array(paths.iter().map(Value::string).collect()),
            ),
        ],
    );
    let checkpoint = result.get("checkpoint").expect("missing capture receipt");
    checkpoint
        .get("ref")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{result:?}"))
        .into()
}

fn restore(tools: &Tools, reference: &str, paths: &[String]) -> Value {
    let mut fields = vec![
        ("action", Value::string("restore")),
        ("ref", Value::string(reference)),
        ("expected", Value::string("latest")),
    ];
    // The whole scope is the default restore selection.
    if paths != ["."] {
        fields.push((
            "paths",
            Value::Array(paths.iter().map(Value::string).collect()),
        ));
    }
    let result = call(tools, "checkpoint", fields);
    for key in ["conflicts", "failed"] {
        assert!(
            result
                .get(key)
                .and_then(Value::as_array)
                .is_none_or(|items| items.is_empty()),
            "{result:?}"
        );
    }
    result
}

fn bytes(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .map(|entry| {
            let entry = entry.unwrap();
            let metadata = entry.metadata().unwrap();
            if metadata.is_dir() {
                bytes(&entry.path())
            } else {
                metadata.len()
            }
        })
        .sum()
}

#[test]
#[ignore = "release benchmark: JECODE_BENCH_CASE, JECODE_BENCH_COUNT, --nocapture --test-threads=1"]
fn matched_tools() {
    let case = std::env::var("JECODE_BENCH_CASE").expect("JECODE_BENCH_CASE");
    let count: usize = std::env::var("JECODE_BENCH_COUNT")
        .unwrap_or_else(|_| "20".into())
        .parse()
        .unwrap();
    let enabled = std::env::var("JECODE_BENCH_ENABLED").map_or(true, |v| v != "0");
    if case == "large256m" {
        return large(count, enabled);
    }
    let home = Directory::new();
    let root = Directory::new();
    let files = if case.ends_with("1000") {
        1000
    } else if case.ends_with("100") {
        100
    } else {
        1
    };
    let length = if case == "edit1m" { 1024 * 1024 } else { 4096 };
    let mut original = vec![b'x'; length];
    original[..5].copy_from_slice(b"alpha");
    let changed = vec![b'y'; length];
    let names: Vec<String> = (0..files).map(|n| format!("file-{n:04}.txt")).collect();
    for name in &names {
        fs::write(root.path().join(name), &original).unwrap();
    }
    let mut tools = Tools::new(root.path()).unwrap();
    tools.configure_output(
        home.path().join("outputs"),
        crate::redact::Redactor::empty(),
    );
    if enabled {
        let session = Session::offline(home.path(), tools.root());
        session.begin("benchmark");
        tools.configure_checkpoints(session);
    }
    let scope = if files > 1 {
        vec![".".into()]
    } else {
        names.clone()
    };
    for sample in 0..count + 2 {
        for name in &names {
            fs::write(root.path().join(name), &original).unwrap();
        }
        let reference = if case.starts_with("restore") {
            Some(capture(&tools, &scope))
        } else {
            None
        };
        if reference.is_some() {
            for name in &names {
                fs::write(root.path().join(name), &changed).unwrap();
            }
        }
        let started = Instant::now();
        let result = match case.as_str() {
            "write4k" => call(
                &tools,
                "write",
                vec![
                    ("path", Value::string(&names[0])),
                    (
                        "content",
                        Value::string(std::str::from_utf8(&changed).unwrap()),
                    ),
                ],
            ),
            "edit1m" => call(
                &tools,
                "edit",
                vec![
                    ("path", Value::string(&names[0])),
                    ("old_text", Value::string("alpha")),
                    ("new_text", Value::string("omega")),
                ],
            ),
            name if name.starts_with("bash") => call(
                &tools,
                "bash",
                vec![
                    ("command", Value::string(":")),
                    (
                        "protect",
                        Value::Array(scope.iter().map(Value::string).collect()),
                    ),
                ],
            ),
            name if name.starts_with("restore") => {
                restore(&tools, reference.as_ref().unwrap(), &scope)
            }
            _ => panic!("Unknown benchmark case {case}"),
        };
        let elapsed = started.elapsed().as_nanos();
        // Verify exact results outside the measured interval.
        let mut expected = if case == "write4k" {
            changed.clone()
        } else {
            original.clone()
        };
        if case == "edit1m" {
            expected[..5].copy_from_slice(b"omega");
        }
        for name in &names {
            assert_eq!(
                fs::read(root.path().join(name)).unwrap(),
                expected,
                "{case}: {result:?}"
            );
        }
        if enabled && !case.starts_with("restore") {
            let receipt = result
                .get("checkpoint")
                .unwrap_or_else(|| panic!("{result:?}"));
            assert!(
                receipt.get("ref").and_then(Value::as_str).is_some(),
                "{result:?}"
            );
        }
        if sample >= 2 {
            println!("BENCH,{case},{enabled},{},{elapsed}", sample - 2);
        }
    }
    tools.stop_processes();
    println!("FOOTPRINT,{case},{enabled},{}", bytes(home.path()));
    println!(
        "ARCHIVE,{case},{enabled},{}",
        bytes(&home.path().join("checkpoints"))
    );
}

fn large(count: usize, enabled: bool) {
    let home = Directory::new();
    let root = Directory::new();
    let path = root.path().join("large.txt");
    let mut tools = Tools::new(root.path()).unwrap();
    tools.configure_output(
        home.path().join("outputs"),
        crate::redact::Redactor::empty(),
    );
    if enabled {
        let session = Session::offline(home.path(), tools.root());
        session.begin("benchmark");
        tools.configure_checkpoints(session);
    }
    for sample in 0..count + 2 {
        let mut file = fs::File::create(&path).unwrap();
        let block = [b'x'; 64 * 1024];
        file.write_all(b"alpha").unwrap();
        for _ in 0..4095 {
            file.write_all(&block).unwrap();
        }
        file.write_all(&block[..block.len() - 5]).unwrap();
        drop(file);
        let started = Instant::now();
        let result = call(
            &tools,
            "edit",
            vec![
                ("path", Value::string("large.txt")),
                ("old_text", Value::string("alpha")),
                ("new_text", Value::string("omega")),
            ],
        );
        let elapsed = started.elapsed().as_nanos();
        let mut file = fs::File::open(&path).unwrap();
        let mut prefix = [0; 5];
        file.read_exact(&mut prefix).unwrap();
        assert_eq!(&prefix, b"omega", "{result:?}");
        let mut buffer = [0; 64 * 1024];
        let mut length = 5;
        loop {
            let n = file.read(&mut buffer).unwrap();
            if n == 0 {
                break;
            }
            assert!(buffer[..n].iter().all(|byte| *byte == b'x'));
            length += n;
        }
        assert_eq!(length, 256 * 1024 * 1024);
        if sample >= 2 {
            println!("BENCH,large256m,{enabled},{},{elapsed}", sample - 2);
        }
    }
    println!("FOOTPRINT,large256m,{enabled},{}", bytes(home.path()));
    println!(
        "ARCHIVE,large256m,{enabled},{}",
        bytes(&home.path().join("checkpoints"))
    );
}
