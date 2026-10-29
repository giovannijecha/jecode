use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static LOW_SPACE_PROBES: AtomicUsize = AtomicUsize::new(0);
static PATH_PROBES: AtomicUsize = AtomicUsize::new(0);

fn counted_low_space(_: &Path, _: Option<&Cancellation>) -> Result<u64, String> {
    LOW_SPACE_PROBES.fetch_add(1, Ordering::SeqCst);
    Ok(1024)
}

fn free_1k(_: &Path, _: Option<&Cancellation>) -> Result<u64, String> {
    Ok(1024)
}

fn fail_first_archive(path: &Path, _: Option<&Cancellation>) -> Result<u64, String> {
    PATH_PROBES.fetch_add(1, Ordering::SeqCst);
    if path == Path::new("removed-archive") {
        Err("archive disappeared".into())
    } else {
        Ok(1024)
    }
}

fn settled(free: u64) -> Arc<Pressure> {
    Arc::new(Pressure {
        path: PathBuf::from("unused"),
        probe: free_1k,
        sample: Mutex::new(Sample::Ready(Estimate {
            free,
            sampled: Instant::now(),
            low_probe: false,
            refreshing: false,
            spent_during_refresh: 0,
        })),
        changed: Condvar::new(),
    })
}

#[test]
fn claims_debit_free_space_and_refresh_when_low() {
    let pressure = settled(1000);
    pressure.claim(Path::new("unused"), 400, 256, None).unwrap();
    assert!(matches!(
        *pressure.sample.lock().unwrap(),
        Sample::Ready(Estimate { free: 600, .. })
    ));
    // The cached estimate is low; a fresh sample permits another write.
    pressure.claim(Path::new("unused"), 400, 256, None).unwrap();
    assert!(matches!(
        *pressure.sample.lock().unwrap(),
        Sample::Ready(Estimate { free: 624, .. })
    ));
    let error = pressure
        .claim(Path::new("unused"), 900, 256, None)
        .unwrap_err();
    assert!(error.contains("insufficient free disk space"), "{error}");
}

#[test]
fn repeated_insufficient_claims_reuse_recent_probe() {
    LOW_SPACE_PROBES.store(0, Ordering::SeqCst);
    let path = Path::new("unused");
    let pressure = Pressure::for_test(path, counted_low_space);
    assert!(pressure.claim(path, 900, 256, None).is_err());
    let fresh = LOW_SPACE_PROBES.load(Ordering::SeqCst);
    assert!(fresh >= 1);
    for _ in 0..4 {
        assert!(pressure.claim(path, 900, 256, None).is_err());
    }
    assert_eq!(LOW_SPACE_PROBES.load(Ordering::SeqCst), fresh);

    if let Sample::Ready(estimate) = &mut *pressure.sample.lock().unwrap() {
        estimate.sampled -= MAX_SAMPLE_AGE;
    }
    assert!(pressure.claim(path, 900, 256, None).is_err());
    assert!(LOW_SPACE_PROBES.load(Ordering::SeqCst) > fresh);
}

#[test]
fn waiting_for_an_initial_sample_observes_cancellation() {
    let pressure = Arc::new(Pressure {
        path: PathBuf::from("unused"),
        probe: free_1k,
        sample: Mutex::new(Sample::Pending),
        changed: Condvar::new(),
    });
    let cancellation = Cancellation::default();
    let stop = cancellation.clone();
    let worker = thread::spawn(move || pressure.claim(Path::new("unused"), 1, 1, Some(&stop)));
    thread::sleep(Duration::from_millis(10));
    cancellation.cancel();
    assert_eq!(worker.join().unwrap().unwrap_err(), "Operation cancelled");
}

#[test]
fn transient_probe_failure_can_recover_after_the_sample_interval() {
    let pressure = settled(0);
    pressure.set_sample(Path::new("unused"), Err("temporary query failure".into()));
    assert!(
        pressure
            .claim(Path::new("unused"), 1, 1, None)
            .unwrap_err()
            .contains("temporary query failure")
    );
    if let Sample::Failed { since, .. } = &mut *pressure.sample.lock().unwrap() {
        *since -= MAX_SAMPLE_AGE;
    }
    pressure.claim(Path::new("unused"), 400, 256, None).unwrap();
    assert!(matches!(
        *pressure.sample.lock().unwrap(),
        Sample::Ready(Estimate { free: 624, .. })
    ));
}

#[test]
fn failed_archive_does_not_throttle_another_archive_on_the_same_volume() {
    PATH_PROBES.store(0, Ordering::SeqCst);
    let first = Path::new("removed-archive");
    let second = Path::new("live-archive");
    let pressure = Pressure::for_test(first, fail_first_archive);

    assert!(pressure.claim(first, 1, 1, None).is_err());
    assert_eq!(PATH_PROBES.load(Ordering::SeqCst), 1);
    assert!(pressure.claim(first, 1, 1, None).is_err());
    assert_eq!(PATH_PROBES.load(Ordering::SeqCst), 1);

    pressure.claim(second, 1, 1, None).unwrap();
    assert_eq!(PATH_PROBES.load(Ordering::SeqCst), 2);
}

#[cfg(windows)]
#[test]
fn native_probe_reads_space_at_an_existing_directory() {
    let archive = std::fs::canonicalize(std::env::temp_dir()).unwrap();
    assert!(sample_free(&archive, None).unwrap() > 0);
}
