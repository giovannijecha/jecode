//! Run with `cargo test --release --locked --offline large_batch_edit_measurement -- --ignored --nocapture`.
//! This opt-in measurement uses the public edit path and keeps fixture creation
//! and exact output verification outside the timed operation.
use super::super::*;
use crate::test_support::Directory;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::time::Instant;

const FILE_BYTES: usize = 32 * 1024 * 1024;
const EDITS: usize = 10;
const WARMUPS: usize = 2;
const SAMPLES: usize = 5;

fn markers() -> Vec<(String, String)> {
    (0..EDITS)
        .map(|index| {
            (
                format!("<jecode-measure-{index:02}-old>"),
                format!(
                    "<jecode-measure-{index:02}-new:{}>",
                    "z".repeat(7 + index * 17)
                ),
            )
        })
        .collect()
}

fn fill(writer: &mut impl Write, mut bytes: usize) {
    static BLOCK: [u8; 64 * 1024] = [b'x'; 64 * 1024];
    while bytes > 0 {
        let count = bytes.min(BLOCK.len());
        writer.write_all(&BLOCK[..count]).unwrap();
        bytes -= count;
    }
}

fn fixture(path: &Path, replacements: &[(String, String)], replaced: bool) {
    let mut writer = BufWriter::new(File::create(path).unwrap());
    let segment = FILE_BYTES / EDITS;
    for (old, new) in replacements {
        fill(&mut writer, segment - old.len());
        writer
            .write_all(if replaced {
                new.as_bytes()
            } else {
                old.as_bytes()
            })
            .unwrap();
    }
    fill(&mut writer, FILE_BYTES % EDITS);
    writer.flush().unwrap();
}

fn assert_same_file(actual: &Path, expected: &Path) {
    let mut remaining = fs::metadata(expected).unwrap().len();
    assert_eq!(fs::metadata(actual).unwrap().len(), remaining);
    let mut actual = BufReader::new(File::open(actual).unwrap());
    let mut expected = BufReader::new(File::open(expected).unwrap());
    let mut left = [0; 64 * 1024];
    let mut right = [0; 64 * 1024];
    while remaining > 0 {
        let count = remaining.min(left.len() as u64) as usize;
        actual.read_exact(&mut left[..count]).unwrap();
        expected.read_exact(&mut right[..count]).unwrap();
        assert_eq!(&left[..count], &right[..count]);
        remaining -= count as u64;
    }
}

#[test]
#[ignore = "opt-in 32 MiB, seven-run performance measurement"]
fn large_batch_edit_measurement() {
    let directory = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    let original = root.join("original.txt");
    let expected = root.join("expected.txt");
    let source = root.join("source.txt");
    let replacements = markers();
    fixture(&original, &replacements, false);
    fixture(&expected, &replacements, true);
    assert_eq!(fs::metadata(&original).unwrap().len(), FILE_BYTES as u64);
    let expected_bytes = fs::metadata(&expected).unwrap().len();
    let arguments = Value::object([
        ("path", Value::string("source.txt")),
        (
            "edits",
            Value::Array(
                replacements
                    .iter()
                    .map(|(old, new)| {
                        Value::object([
                            ("old_text", Value::string(old.clone())),
                            ("new_text", Value::string(new.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]);

    let mut samples_ms = Vec::with_capacity(SAMPLES);
    for run in 0..WARMUPS + SAMPLES {
        fs::copy(&original, &source).unwrap();
        let started = Instant::now();
        let (result, change) =
            edit_with_change(&root, &arguments, None, &Cancellation::default()).unwrap();
        let elapsed = started.elapsed();
        assert_eq!(
            result.get("bytes_written"),
            Some(&Value::number(expected_bytes))
        );
        assert!(change.is_none());
        assert_same_file(&source, &expected);
        if run >= WARMUPS {
            samples_ms.push(elapsed.as_secs_f64() * 1_000.0);
        }
    }
    let mut sorted = samples_ms.clone();
    sorted.sort_by(f64::total_cmp);
    println!(
        "edits={EDITS} file_bytes={FILE_BYTES} warmups={WARMUPS} samples_ms={samples_ms:?} median_ms={}",
        sorted[SAMPLES / 2]
    );
}
