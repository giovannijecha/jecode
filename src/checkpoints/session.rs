//! Per-call recovery at the tool boundary. A receipt describes only the
//! preimage durably captured before the tool was allowed to write.
use super::locks::Lease;
use super::paths;
use super::store::Store;
use super::types::{Capture, TTL_MILLIS};
use crate::cancel::Cancellation;
use crate::json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const PARTIAL_LIMIT: usize = 20;

pub struct Session {
    store: Result<Arc<Store>, String>,
    origin: Mutex<String>,
    instructions_changed: Mutex<bool>,
    _maintenance: Option<Maintenance>,
}

struct Maintenance {
    stop: Cancellation,
    worker: Option<JoinHandle<()>>,
}

impl Maintenance {
    fn start(store: &Arc<Store>, home: PathBuf) -> Self {
        let stop = Cancellation::default();
        let cancellation = stop.clone();
        let store = Arc::downgrade(store);
        let worker = thread::spawn(move || {
            loop {
                if cancellation.requested() {
                    break;
                }
                let Some(active) = store.upgrade() else {
                    break;
                };
                drop(active);
                let _ = Store::sweep_home(&home, super::clock::now(), &cancellation);
                if cancellation.wait(Duration::from_secs(60)).is_err() {
                    break;
                }
            }
        });
        Self {
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for Maintenance {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Session {
    pub fn open(home: &Path, root: &Path) -> Arc<Self> {
        Self::open_with_maintenance(home, root, true)
    }

    fn open_with_maintenance(home: &Path, root: &Path, maintain: bool) -> Arc<Self> {
        let store = Store::open(home, root)
            .map_err(|error| format!("checkpoints are unavailable: {error}"));
        let maintenance = if maintain {
            store
                .as_ref()
                .ok()
                .map(|store| Maintenance::start(store, home.to_path_buf()))
        } else {
            None
        };
        Arc::new(Self {
            store,
            origin: Mutex::new(String::new()),
            instructions_changed: Mutex::new(false),
            _maintenance: maintenance,
        })
    }

    #[cfg(test)]
    pub fn offline(home: &Path, root: &Path) -> Arc<Self> {
        Self::open_with_maintenance(home, root, false)
    }

    pub fn store(&self) -> Result<&Arc<Store>, String> {
        self.store.as_ref().map_err(Clone::clone)
    }

    pub fn origin(&self) -> String {
        self.origin.lock().unwrap().clone()
    }

    pub fn begin(&self, origin: &str) {
        *self.origin.lock().unwrap() = origin.into();
        *self.instructions_changed.lock().unwrap() = false;
    }

    pub fn restored_instructions(&self) {
        *self.instructions_changed.lock().unwrap() = true;
    }

    pub fn take_restored_instructions(&self) -> bool {
        std::mem::take(&mut *self.instructions_changed.lock().unwrap())
    }

    pub fn file_cancellable<'a>(
        &'a self,
        kind: &'static str,
        path: &Path,
        cancellation: &'a Cancellation,
    ) -> FileGuard<'a> {
        let prepared = self.store().and_then(|store| {
            let relative = path
                .strip_prefix(&store.root)
                .map_err(|_| "File is outside the recovery workspace".to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            let lease = store
                .leases
                .acquire_cancellable(&[&relative], cancellation)
                .map_err(|error| error.to_string())?;
            Ok((relative, lease))
        });
        FileGuard {
            session: self,
            kind,
            cancellation,
            prepared,
            capture: None,
        }
    }

    pub fn command_scoped_cancellable(
        &self,
        paths: &[String],
        cancellation: &Cancellation,
    ) -> CommandGuard {
        let capture = if paths.is_empty() {
            Ok(Capture::default())
        } else {
            self.store().and_then(|store| {
                let exact = exact_paths(store, paths);
                let lease = match &exact {
                    Some(names) => store.leases.acquire_cancellable(
                        &names.iter().map(String::as_str).collect::<Vec<_>>(),
                        cancellation,
                    ),
                    // A recursive scope can contain any stripe.
                    None => store.leases.acquire_all_cancellable(cancellation),
                }
                .map_err(|error| error.to_string())?;
                let capture = store.capture("bash", &self.origin(), paths, cancellation);
                drop(lease);
                capture
            })
        };
        let receipt = self.receipt(capture);
        CommandGuard { receipt }
    }

    fn receipt(&self, capture: Result<Capture, String>) -> Value {
        let capture = match capture {
            Ok(capture) => capture,
            Err(error) => {
                return Value::object([
                    ("status", Value::string("none")),
                    ("saved_paths", Value::number(0)),
                    ("reason", Value::string(error)),
                ]);
            }
        };
        let status = if capture.reference.is_none() {
            "none"
        } else if capture.partial.is_empty() {
            "saved"
        } else {
            "partial"
        };
        let mut fields = vec![
            ("status", Value::string(status)),
            ("saved_paths", Value::number(capture.saved)),
        ];
        if let Some(reference) = &capture.reference {
            fields.push(("ref", Value::string(reference)));
            fields.push((
                "expires_at",
                Value::number(capture.created.saturating_add(TTL_MILLIS)),
            ));
        }
        if !capture.partial.is_empty() {
            fields.push((
                "partial",
                Value::Array(
                    capture
                        .partial
                        .iter()
                        .take(PARTIAL_LIMIT)
                        .map(|(path, reason)| {
                            Value::object([
                                ("path", Value::string(path)),
                                ("reason", Value::string(reason)),
                            ])
                        })
                        .collect(),
                ),
            ));
            if capture.partial.len() > PARTIAL_LIMIT {
                fields.push(("partial_count", Value::number(capture.partial.len())));
            }
        }
        Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key.into(), value))
                .collect(),
        )
    }
}

/// An exact file, absence or final link needs only its normalized path stripe.
/// Directories and uncertain paths keep full coverage during discovery.
fn exact_paths(store: &Store, requested: &[String]) -> Option<Vec<String>> {
    let mut names = Vec::with_capacity(requested.len());
    for request in requested {
        if request.is_empty() || request == "." || request.ends_with('/') {
            return None;
        }
        let absolute = if Path::new(request).is_absolute() {
            PathBuf::from(request)
        } else {
            paths::absolute(&store.root, request)
        };
        let name = paths::relative(&store.root, &absolute)?;
        match fs::symlink_metadata(paths::absolute(&store.root, &name)) {
            Ok(metadata) if recursive_directory(&metadata) => return None,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return None,
        }
        names.push(name);
    }
    Some(names)
}

fn recursive_directory(metadata: &fs::Metadata) -> bool {
    metadata.is_dir() && !crate::disk::linked(metadata)
}

pub struct FileGuard<'a> {
    session: &'a Session,
    kind: &'static str,
    cancellation: &'a Cancellation,
    prepared: Result<(String, Lease), String>,
    capture: Option<Value>,
}

impl FileGuard<'_> {
    pub fn check_cancelled(&self) -> Result<(), String> {
        if self.cancellation.requested() {
            Err("Operation cancelled".into())
        } else {
            Ok(())
        }
    }

    pub fn capture(&mut self) {
        if self.capture.is_some() {
            return;
        }
        let capture = match &self.prepared {
            Ok((path, _lease)) => self.session.store().and_then(|store| {
                store.capture(
                    self.kind,
                    &self.session.origin(),
                    std::slice::from_ref(path),
                    self.cancellation,
                )
            }),
            Err(error) => Err(error.clone()),
        };
        self.capture = Some(self.session.receipt(capture));
    }

    pub fn finish(self) -> Value {
        self.capture.unwrap_or_else(|| {
            Value::object([
                ("status", Value::string("none")),
                ("saved_paths", Value::number(0)),
            ])
        })
    }
}

pub struct CommandGuard {
    receipt: Value,
}

impl CommandGuard {
    pub fn note(&self) -> String {
        let status = self
            .receipt
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("none");
        let reference = self
            .receipt
            .get("ref")
            .and_then(Value::as_str)
            .unwrap_or("none");
        format!("checkpoint {reference} · {status}")
    }

    pub fn finish(self) -> Value {
        self.receipt
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Directory;

    #[test]
    fn bash_exact_files_use_path_leases_and_directories_use_all_stripes() {
        let home = Directory::new();
        let root = Directory::new();
        fs::write(root.path().join("file.txt"), "before").unwrap();
        fs::create_dir(root.path().join("folder")).unwrap();
        let store = Store::open(home.path(), root.path()).unwrap();
        assert_eq!(
            exact_paths(&store, &["file.txt".into()]),
            Some(vec!["file.txt".into()])
        );
        assert_eq!(
            exact_paths(&store, &["missing.txt".into()]),
            Some(vec!["missing.txt".into()])
        );
        assert!(exact_paths(&store, &["folder".into()]).is_none());
        assert!(exact_paths(&store, &["future/".into()]).is_none());
        assert!(exact_paths(&store, &[".".into()]).is_none());
    }
}
