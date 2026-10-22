use super::*;
use crate::test_support::Directory;

fn grown_files(root: &Path) -> Vec<String> {
    let names: Vec<_> = (0..40).map(|index| format!("f{index:02}.bin")).collect();
    for name in &names {
        fs::write(root.join(name), b"small at discovery").unwrap();
    }
    for (index, name) in names.iter().enumerate() {
        fs::write(root.join(name), vec![index as u8; 512 << 10]).unwrap();
    }
    names
}

#[test]
fn grown_live_files_are_delivered_in_bounded_ordered_batches() {
    let directory = Directory::new();
    let paths = grown_files(directory.path());
    let mut next = 0;
    let mut batches = 0;
    observe_batches(directory.path(), &paths, 10 << 20, |batch| {
        let bytes: usize = batch
            .iter()
            .map(|observed| match &observed.kind {
                Observation::File(bytes, _) => bytes.len(),
                _ => panic!("a live file was not read"),
            })
            .sum();
        assert!(bytes <= BATCH_BYTES as usize + (512 << 10));
        for observed in batch {
            assert_eq!(observed.path, paths[next]);
            let Observation::File(bytes, _) = observed.kind else {
                unreachable!()
            };
            assert_eq!(bytes.len(), 512 << 10);
            assert!(bytes.iter().all(|byte| *byte == next as u8));
            next += 1;
        }
        batches += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(next, paths.len());
    assert!(batches > 1, "the entire live scope was buffered at once");
}

#[test]
fn a_consumer_error_releases_blocked_read_workers() {
    let directory = Directory::new();
    let paths = grown_files(directory.path());
    let mut batches = 0;
    let result = observe_batches(directory.path(), &paths, 10 << 20, |_| {
        batches += 1;
        Err("archive capacity reached".into())
    });
    assert_eq!(result, Err("archive capacity reached".into()));
    assert_eq!(batches, 1);
    // All scoped readers have stopped before the error is returned.
    for name in paths {
        fs::remove_file(directory.path().join(name)).unwrap();
    }
}

#[test]
fn serial_observation_allows_one_file_larger_than_a_batch() {
    let directory = Directory::new();
    fs::write(directory.path().join("large.bin"), vec![7; 9 << 20]).unwrap();
    let mut observed = 0;
    observe_batches(directory.path(), &["large.bin"], 10 << 20, |batch| {
        assert_eq!(batch.len(), 1);
        let Observation::Stream(source, _) = &batch[0].kind else {
            panic!("the file should be streamed without buffering its contents");
        };
        assert_eq!(source.length(), 9 << 20);
        let mut file = source.open().unwrap();
        let mut bytes = [0; 64 << 10];
        use std::io::Read;
        while file.read(&mut bytes).unwrap() != 0 {
            assert!(bytes.iter().all(|byte| *byte == 7));
        }
        source.verify(&file).unwrap();
        observed += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(observed, 1);
}

#[test]
fn streamed_observation_rejects_a_displaced_live_path() {
    let directory = Directory::new();
    let path = directory.path().join("large.bin");
    let mut output = fs::File::create(&path).unwrap();
    output.set_len(INLINE_BYTES + 1).unwrap();
    use std::io::Write;
    output.write_all(b"original").unwrap();
    drop(output);
    let observed = observe(directory.path(), "large.bin", u64::MAX);
    let Observation::Stream(source, _) = observed.kind else {
        panic!("expected streaming observation");
    };
    let handle = source.open().unwrap();
    fs::rename(&path, directory.path().join("displaced.bin")).unwrap();
    fs::write(&path, b"a new live file").unwrap();
    assert!(source.verify(&handle).is_err());
    assert!(source.open().is_err());
}
