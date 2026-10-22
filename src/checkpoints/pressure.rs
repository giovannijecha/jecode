//! Advisory free-space admission for checkpoint journal writes and compaction.
//! The operating system remains the final authority if another process fills
//! the volume after a sample; journal I/O errors must still be reported.
use crate::cancel::Cancellation;
use std::collections::HashMap;
#[cfg(unix)]
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

const PROBE_LIMIT: Duration = Duration::from_secs(10);
const MAX_SAMPLE_AGE: Duration = Duration::from_secs(5);
pub const DEFAULT_MIN_FREE_BYTES: u64 = 256 << 20;
type Probe = fn(&Path, Option<&Cancellation>) -> Result<u64, String>;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum Volume {
    #[cfg(windows)]
    Windows(PathBuf),
    #[cfg(unix)]
    Unix(u64),
}

enum Sample {
    Pending,
    Ready(Estimate),
    Failed { error: String, since: Instant },
}

struct Estimate {
    free: u64,
    sampled: Instant,
    refreshing: bool,
    spent_during_refresh: u64,
}

/// One estimate per volume in this process, shared by open journals. The
/// first sample begins in the background during journal opening.
pub struct Pressure {
    path: PathBuf,
    probe: Probe,
    sample: Mutex<Sample>,
    changed: Condvar,
}

static VOLUMES: OnceLock<Mutex<HashMap<Volume, Arc<Pressure>>>> = OnceLock::new();

pub fn for_archive(path: &Path) -> Result<Arc<Pressure>, String> {
    let volume = volume(path)?;
    let volumes = VOLUMES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut volumes = volumes.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(pressure) = volumes.get(&volume) {
        return Ok(Arc::clone(pressure));
    }
    let pressure = Pressure::start(path.to_path_buf(), sample_free);
    volumes.insert(volume, Arc::clone(&pressure));
    Ok(pressure)
}

impl Pressure {
    #[cfg(test)]
    pub(crate) fn for_test(path: &Path, probe: Probe) -> Arc<Self> {
        Self::start(path.to_path_buf(), probe)
    }

    fn start(path: PathBuf, probe: Probe) -> Arc<Self> {
        let pressure = Arc::new(Self {
            path,
            probe,
            sample: Mutex::new(Sample::Pending),
            changed: Condvar::new(),
        });
        let worker = Arc::clone(&pressure);
        if let Err(error) = thread::Builder::new()
            .name("jecode-checkpoint-space".into())
            .spawn(move || worker.finish_probe())
        {
            pressure.set_sample(Err(format!(
                "Could not start checkpoint disk-space probe: {error}"
            )));
        }
        pressure
    }

    fn finish_probe(&self) {
        self.set_sample((self.probe)(&self.path, None));
    }

    fn set_sample(&self, result: Result<u64, String>) {
        let mut sample = self
            .sample
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *sample = match result {
            Ok(free) => Sample::Ready(Estimate {
                free,
                sampled: Instant::now(),
                refreshing: false,
                spent_during_refresh: 0,
            }),
            Err(error) => Sample::Failed {
                error,
                since: Instant::now(),
            },
        };
        self.changed.notify_all();
    }

    /// Conservatively debits intended bytes before a journal append. A failed
    /// write may have written a prefix, so the debit is not refunded.
    pub fn claim(
        self: &Arc<Self>,
        path: &Path,
        bytes: u64,
        minimum: u64,
        cancellation: Option<&Cancellation>,
    ) -> Result<(), String> {
        let needed = minimum
            .checked_add(bytes)
            .ok_or("Checkpoint disk-space requirement overflows byte accounting")?;
        let mut refreshed = false;
        loop {
            if cancellation.is_some_and(Cancellation::requested) {
                return Err("Operation cancelled".into());
            }
            let mut sample = self
                .sample
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match &mut *sample {
                Sample::Pending => {
                    drop(
                        self.changed
                            .wait_timeout(sample, Duration::from_millis(25))
                            .unwrap_or_else(|error| error.into_inner()),
                    );
                }
                Sample::Failed { error, since } => {
                    if since.elapsed() < MAX_SAMPLE_AGE {
                        return Err(format!("Could not check checkpoint disk space: {error}"));
                    }
                    // A temporary OS query failure must not disable every
                    // archive sharing this estimate until the process exits.
                    *sample = Sample::Pending;
                    drop(sample);
                    self.set_sample((self.probe)(path, cancellation));
                    refreshed = true;
                }
                Sample::Ready(estimate) => {
                    if estimate.sampled.elapsed() >= MAX_SAMPLE_AGE && !estimate.refreshing {
                        estimate.refreshing = true;
                        estimate.spent_during_refresh = 0;
                        drop(sample);
                        self.refresh_in_background(path.to_path_buf());
                        continue;
                    }
                    if estimate.free >= needed {
                        estimate.free -= bytes;
                        if estimate.refreshing {
                            estimate.spent_during_refresh =
                                estimate.spent_during_refresh.saturating_add(bytes);
                        }
                        return Ok(());
                    }
                    if estimate.refreshing {
                        drop(
                            self.changed
                                .wait_timeout(sample, Duration::from_millis(25))
                                .unwrap_or_else(|error| error.into_inner()),
                        );
                        continue;
                    }
                    if refreshed {
                        return Err(insufficient(estimate.free, bytes, minimum));
                    }
                    let prior = estimate.free;
                    *sample = Sample::Pending;
                    drop(sample);
                    let result = (self.probe)(path, cancellation);
                    if cancellation.is_some_and(Cancellation::requested) {
                        self.set_sample(Ok(prior));
                        return Err("Operation cancelled".into());
                    }
                    self.set_sample(result);
                    refreshed = true;
                }
            }
        }
    }

    fn refresh_in_background(self: &Arc<Self>, path: PathBuf) {
        let pressure = Arc::clone(self);
        if let Err(error) = thread::Builder::new()
            .name("jecode-checkpoint-space-refresh".into())
            .spawn(move || {
                let result = (pressure.probe)(&path, None);
                let mut sample = pressure
                    .sample
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                *sample = match result {
                    Ok(free) => {
                        let spent = match &*sample {
                            Sample::Ready(estimate) => estimate.spent_during_refresh,
                            _ => 0,
                        };
                        Sample::Ready(Estimate {
                            free: free.saturating_sub(spent),
                            sampled: Instant::now(),
                            refreshing: false,
                            spent_during_refresh: 0,
                        })
                    }
                    Err(error) => Sample::Failed {
                        error,
                        since: Instant::now(),
                    },
                };
                pressure.changed.notify_all();
            })
        {
            self.set_sample(Err(format!(
                "Could not start checkpoint disk-space probe: {error}"
            )));
        }
    }

    /// Uses a fresh OS sample before creating a second archive generation.
    /// Incremental appends still call `claim` because writers can extend the
    /// old generation while compaction copies it.
    pub fn ensure_scratch(&self, path: &Path, projected: u64, minimum: u64) -> Result<(), String> {
        let needed = minimum
            .checked_add(projected)
            .ok_or("Checkpoint disk-space requirement overflows byte accounting")?;
        let free = (self.probe)(path, None)
            .map_err(|error| format!("Could not check checkpoint disk space: {error}"))?;
        if free >= needed {
            Ok(())
        } else {
            Err(insufficient(free, projected, minimum))
        }
    }
}

fn insufficient(free: u64, operation: u64, minimum: u64) -> String {
    format!(
        "Checkpoint archive has insufficient free disk space: {free} B available, \
         {operation} B requested plus {minimum} B minimum free"
    )
}

#[cfg(windows)]
fn volume(path: &Path) -> Result<Volume, String> {
    // A drive prefix is not a volume identity: C:\mnt may be another mounted
    // volume, and an archive may live on a UNC share. Share the estimate only
    // among journals for the same resolved archive directory.
    std::fs::canonicalize(path)
        .map(Volume::Windows)
        .map_err(|error| format!("Could not locate checkpoint archive volume: {error}"))
}

#[cfg(unix)]
fn volume(path: &Path) -> Result<Volume, String> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path)
        .map(|metadata| Volume::Unix(metadata.dev()))
        .map_err(|error| format!("Could not locate checkpoint archive volume: {error}"))
}

fn run(mut command: Command, cancellation: Option<&Cancellation>) -> Result<String, String> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not start system disk-space query: {error}"))?;
    let started = Instant::now();
    loop {
        if cancellation.is_some_and(Cancellation::requested) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Operation cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < PROBE_LIMIT => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("system disk-space query timed out".into());
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("system disk-space query failed: {error}"));
            }
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("could not read system disk-space query: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "system disk-space query exited with {}: {}",
            output.status,
            detail.trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|_| "system disk-space query returned non-UTF-8 output".into())
}

#[cfg(windows)]
fn sample_free(path: &Path, cancellation: Option<&Cancellation>) -> Result<u64, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const SOURCE: &str = include_str!("space.cs");
    static HELPER: OnceLock<PathBuf> = OnceLock::new();
    let fallback = Cancellation::default();
    let executable = cached_helper(&HELPER, || {
        crate::process::compile_helper(
            "checkpoint-space-v1",
            SOURCE,
            cancellation.unwrap_or(&fallback),
        )
    })
    .ok_or("Could not compile the checkpoint disk-space helper")?;
    let mut command = Command::new(executable);
    command
        .arg(path)
        .env_remove("OPENROUTER_API_KEY")
        .creation_flags(CREATE_NO_WINDOW);
    run(command, cancellation)?
        .trim()
        .parse()
        .map_err(|_| "system disk-space query returned an invalid byte count".into())
}

#[cfg(windows)]
fn cached_helper(
    cache: &OnceLock<PathBuf>,
    build: impl FnOnce() -> Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(path) = cache.get() {
        return Some(path.clone());
    }
    let path = build()?;
    let _ = cache.set(path.clone());
    Some(path)
}

#[cfg(unix)]
fn sample_free(path: &Path, cancellation: Option<&Cancellation>) -> Result<u64, String> {
    let mut command = Command::new("df");
    command.args(["-Pk"]).arg(path).env("LC_ALL", "C");
    let output = run(command, cancellation)?;
    let row = output
        .lines()
        .nth(1)
        .ok_or("system disk-space query returned no volume row")?;
    let kib = row
        .split_whitespace()
        .nth(3)
        .ok_or("system disk-space query returned no available space")?
        .parse::<u64>()
        .map_err(|_| "system disk-space query returned an invalid byte count")?;
    kib.checked_mul(1024)
        .ok_or("system disk-space query byte count overflows")
        .map_err(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn free_1k(_: &Path, _: Option<&Cancellation>) -> Result<u64, String> {
        Ok(1024)
    }

    fn settled(free: u64) -> Arc<Pressure> {
        Arc::new(Pressure {
            path: PathBuf::from("unused"),
            probe: free_1k,
            sample: Mutex::new(Sample::Ready(Estimate {
                free,
                sampled: Instant::now(),
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
    fn compaction_requires_scratch_beside_the_free_space_minimum() {
        let pressure = settled(10);
        pressure
            .ensure_scratch(Path::new("unused"), 700, 256)
            .unwrap();
        let error = pressure
            .ensure_scratch(Path::new("unused"), 800, 256)
            .unwrap_err();
        assert!(error.contains("800 B requested plus 256 B"), "{error}");
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
        pressure.set_sample(Err("temporary query failure".into()));
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

    #[cfg(windows)]
    #[test]
    fn failed_helper_compilation_is_not_cached() {
        let cache = OnceLock::new();
        assert!(cached_helper(&cache, || None).is_none());
        let expected = PathBuf::from("compiled-helper.exe");
        assert_eq!(
            cached_helper(&cache, || Some(expected.clone())),
            Some(expected.clone())
        );
        assert_eq!(
            cached_helper(&cache, || panic!("already compiled")),
            Some(expected)
        );
    }

    #[cfg(windows)]
    #[test]
    fn native_probe_reads_space_at_an_existing_directory() {
        let archive = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        assert!(sample_free(&archive, None).unwrap() > 0);
    }
}
