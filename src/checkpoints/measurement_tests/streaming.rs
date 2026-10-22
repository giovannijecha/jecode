//! Large-file round trips whose fixtures and comparisons also use bounded RAM.
use super::*;
use crate::checkpoints::mutation::Protection;
use crate::checkpoints::record::State;
use crate::checkpoints::restore::{Expected, Request};
use std::io::{Read, Write};

const LENGTH: u64 = 256 << 20;

struct Generated {
    remaining: u64,
    state: u64,
}

impl Generated {
    fn new(seed: u64) -> Self {
        Self {
            remaining: LENGTH,
            state: seed,
        }
    }
}

impl Read for Generated {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let count = self.remaining.min(output.len() as u64) as usize;
        for byte in &mut output[..count] {
            self.state ^= self.state << 13;
            self.state ^= self.state >> 7;
            self.state ^= self.state << 17;
            *byte = self.state as u8;
        }
        self.remaining -= count as u64;
        Ok(count)
    }
}

fn create(path: &Path, seed: u64) {
    let mut file = fs::File::create(path).unwrap();
    assert_eq!(
        std::io::copy(&mut Generated::new(seed), &mut file).unwrap(),
        LENGTH
    );
    file.flush().unwrap();
}

fn verify(path: &Path, seed: u64) {
    let mut actual = fs::File::open(path).unwrap();
    let mut expected = Generated::new(seed);
    let mut left = [0; 64 * 1024];
    let mut right = [0; 64 * 1024];
    loop {
        let count = expected.read(&mut right).unwrap();
        if count == 0 {
            break;
        }
        actual.read_exact(&mut left[..count]).unwrap();
        assert_eq!(left[..count], right[..count]);
    }
    assert_eq!(actual.read(&mut left[..1]).unwrap(), 0);
}

pub fn run(recorder: &mut Recorder) {
    let root = Directory::new();
    let path = root.path().join("large.bin");
    let replacement = root.path().join(".jecode-replacement.tmp");
    let (mut capture, mut prepare, mut restore, mut undo, mut collect) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for sample in 0..3 {
        create(&path, 100);
        create(&replacement, 200);
        let home = Directory::new();
        let (workspace, elapsed) = timed(|| {
            let workspace = workspace::open(home.path(), root.path(), Options::offline()).unwrap();
            workspace.initialize().unwrap();
            workspace
        });
        capture.push(elapsed);
        let target = workspace
            .mark("milestone", "original", "measure", true, false)
            .unwrap();
        let mut mutation = workspace.mutation("measure", "write", &path);
        prepare.push(timed(|| mutation.prepare_file(&replacement)).1);
        fs::rename(&replacement, &path).unwrap();
        assert!(matches!(
            mutation.finish(true),
            Protection::Protected { .. }
        ));
        let after = workspace
            .mark("milestone", "rewritten", "measure", true, false)
            .unwrap();
        verify(&path, 200);
        let request = |target| Request {
            target,
            paths: None,
            exact: false,
            expected: Expected::Latest,
            preview: false,
            origin: "measure",
            excluded: &workspace.excluded,
        };
        let (report, elapsed) = timed(|| workspace.restore(&request(target)).unwrap());
        assert!(report.failed.is_none() && report.conflicts.is_empty());
        restore.push(elapsed);
        verify(&path, 100);
        let (report, elapsed) =
            timed(|| workspace.restore(&request(report.before.unwrap())).unwrap());
        assert!(report.failed.is_none() && report.conflicts.is_empty());
        undo.push(elapsed);
        verify(&path, 200);
        collect.push(timed(|| workspace.collect(true).unwrap().unwrap()).1);
        {
            let mut store = workspace.store();
            store.refresh().unwrap();
            for (point, seed) in [(target, 100), (after, 200)] {
                let State::File { object, length, .. } = store.index.state_at("large.bin", point)
                else {
                    panic!("large file was not captured");
                };
                assert_eq!(length, LENGTH);
                assert!(store.equal_reader(object, Generated::new(seed)).unwrap());
            }
        }
        if sample == 2 {
            recorder.metric(
                "streaming",
                "two unrelated 256 MiB states",
                "archive bytes",
                archive_bytes(&workspace.archive) as f64,
                "bytes",
            );
        }
        capture::close(workspace);
    }
    for (case, times) in [
        ("256 MiB initial capture", capture),
        ("256 MiB durable replacement preparation", prepare),
        ("256 MiB restore", restore),
        ("256 MiB undo", undo),
        ("two 256 MiB states compaction", collect),
    ] {
        recorder.add(["streaming", case, "total"], "three fresh archives; warm file cache; exact byte verification outside timers; fixtures and comparisons stream", times);
    }
}
