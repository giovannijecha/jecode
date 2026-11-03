use super::Client;
use crate::json::Value;
use crate::lsp::documents::path_to_uri;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_ENTRIES: usize = 10_000;
const MAX_HASHED_BYTES: u64 = 32 * 1024 * 1024;
/// A hash taken this soon after the last write may miss a later same-size
/// write that keeps the timestamp, so it is not reused.
const SETTLED: Duration = Duration::from_secs(2);

/// Last observed state of a watched path. Content is reread only when its
/// size or timestamp changes, or while its hash is not yet settled.
#[derive(Clone, Copy)]
pub(super) struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
    settled: bool,
    hash: u64,
}

impl Client {
    pub(super) fn poll_watches(&mut self) -> Result<(), String> {
        if self.registrations.values().all(Vec::is_empty)
            && self.observed_extensions.is_empty()
            && self.observed_markers.is_empty()
        {
            return Ok(());
        }
        let Some(found) = self.scan_watches() else {
            self.sync_gap = true;
            return Ok(());
        };
        let patterns = self.registrations.values().flatten().collect::<Vec<_>>();
        let creations = self.watch_initialized;
        let changes = found
            .iter()
            .filter_map(|(path, stamp)| match self.watched.get(path) {
                None => creations.then(|| (path.clone(), 1)),
                Some(previous) if previous.hash != stamp.hash => Some((path.clone(), 2)),
                _ => None,
            })
            .chain(
                self.watched
                    .keys()
                    .filter(|path| !found.contains_key(*path) && !path.exists())
                    .map(|path| (path.clone(), 3)),
            )
            .filter(|(path, change)| {
                self.generic_file(path)
                    || patterns.iter().any(|watch| {
                        let mask = match change {
                            1 => 1,
                            2 => 2,
                            _ => 4,
                        };
                        watch.kind & mask != 0 && watch.matches(path)
                    })
            })
            .collect::<Vec<_>>();
        self.watched = found;
        self.sync_gap = false;
        self.watch_initialized = true;
        let dependency_changed = changes
            .iter()
            .any(|(path, _)| !self.docs.values().any(|document| document.path == *path));
        if !changes.is_empty() {
            self.notify(
                "workspace/didChangeWatchedFiles",
                Value::object([(
                    "changes",
                    Value::Array(
                        changes
                            .into_iter()
                            .map(|(path, kind)| {
                                Value::object([
                                    ("uri", Value::string(path_to_uri(&path))),
                                    ("type", Value::number(kind)),
                                ])
                            })
                            .collect(),
                    ),
                )]),
            )?;
        }
        if dependency_changed {
            self.resync_managed_documents()?;
        }
        Ok(())
    }

    /// Every watched path with its stamp, or None when the scan is incomplete.
    fn scan_watches(&self) -> Option<HashMap<PathBuf, Stamp>> {
        let patterns = self.registrations.values().flatten().collect::<Vec<_>>();
        let mut scan = Scan {
            now: SystemTime::now(),
            entries: 0,
            bytes: 0,
        };
        let mut found = HashMap::new();
        for path in patterns.iter().filter_map(|watch| watch.literal.as_ref()) {
            scan.entries += 1;
            if let Some(stamp) = scan.stamp(path, self.watched.get(path), true).ok()? {
                found.insert(path.clone(), stamp);
            }
        }
        let mut stack = vec![self.root.clone()];
        stack.extend(
            patterns
                .iter()
                .filter(|watch| {
                    watch.literal.is_none() && !implicitly_reached(&self.root, &watch.base)
                })
                .map(|watch| watch.base.clone()),
        );
        let mut visited = HashSet::new();
        while let Some(folder) = stack.pop() {
            if !visited.insert(folder.clone()) {
                continue;
            }
            let entries = match std::fs::read_dir(&folder) {
                Ok(entries) => entries,
                // A missing base, or a folder removed mid-scan, holds no paths.
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(_) => return None,
            };
            for entry in entries {
                let entry = entry.ok()?;
                scan.entries += 1;
                if scan.entries > MAX_ENTRIES {
                    return None;
                }
                let path = entry.path();
                let kind = entry.file_type().ok()?;
                if kind.is_dir() {
                    if !skipped(&entry.file_name()) {
                        stack.push(path);
                    }
                    continue;
                }
                if !kind.is_file()
                    || found.contains_key(&path)
                    || (!self.generic_file(&path)
                        && !patterns.iter().any(|watch| watch.matches(&path)))
                {
                    continue;
                }
                if let Some(stamp) = scan.stamp(&path, self.watched.get(&path), false).ok()? {
                    found.insert(path, stamp);
                }
            }
        }
        Some(found)
    }

    fn generic_file(&self, path: &Path) -> bool {
        let associated = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                self.observed_extensions
                    .iter()
                    .any(|known| known == extension)
            });
        let relative = path.strip_prefix(&self.root).ok();
        let marker = relative.is_some_and(|relative| {
            self.observed_markers
                .iter()
                .any(|known| relative == Path::new(known))
        });
        associated || marker
    }
}

/// Generated, dependency and scratch folders are left out of implicit descent;
/// a watch base registered inside one is still scanned as its own root.
fn skipped(name: &OsStr) -> bool {
    name.to_str().is_some_and(|name| {
        name.eq_ignore_ascii_case(".tmp") || crate::checkpoints::generated_name(name)
    })
}

fn implicitly_reached(root: &Path, base: &Path) -> bool {
    base.strip_prefix(root)
        .is_ok_and(|rest| !rest.iter().any(skipped))
}

struct Scan {
    now: SystemTime,
    entries: usize,
    bytes: u64,
}

impl Scan {
    /// None when the path is absent; Err when the scan cannot be completed.
    fn stamp(
        &mut self,
        path: &Path,
        previous: Option<&Stamp>,
        directory: bool,
    ) -> Result<Option<Stamp>, ()> {
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(()),
        };
        let modified = metadata.modified().ok();
        if metadata.is_dir() {
            let hash = modified
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |time| time.as_nanos() as u64);
            return Ok(directory.then_some(Stamp {
                len: 0,
                modified,
                settled: false,
                hash,
            }));
        }
        if !metadata.is_file() {
            return Ok(None);
        }
        let len = metadata.len();
        if let Some(previous) = previous
            && previous.settled
            && previous.len == len
            && modified.is_some()
            && previous.modified == modified
        {
            return Ok(Some(*previous));
        }
        let settled = modified
            .and_then(|time| self.now.duration_since(time).ok())
            .is_some_and(|age| age >= SETTLED);
        Ok(self.hash(path, len)?.map(|hash| Stamp {
            len,
            modified,
            settled,
            hash,
        }))
    }

    fn hash(&mut self, path: &Path, len: u64) -> Result<Option<u64>, ()> {
        let remaining = MAX_HASHED_BYTES.saturating_sub(self.bytes);
        if len > remaining {
            return Err(());
        }
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(()),
        };
        let mut data = Vec::new();
        file.take(remaining + 1)
            .read_to_end(&mut data)
            .map_err(|_| ())?;
        if data.len() as u64 > remaining {
            return Err(());
        }
        self.bytes += data.len() as u64;
        Ok(Some(
            data.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
            }),
        ))
    }
}

#[cfg(test)]
#[path = "sync_tests.rs"]
mod sync_tests;

#[cfg(test)]
fn glob_match(pattern: &str, path: &str) -> bool {
    crate::lsp::glob::Pattern::new(pattern).is_some_and(|pattern| pattern.matches(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cancel::Cancellation;
    use crate::lsp::client::Client;
    use crate::lsp::fixture::Fixture;
    use crate::test_support::Directory;
    use std::time::Duration;

    #[test]
    fn disappearing_watched_file_and_folder_do_not_crash_owned_server() {
        let fixture = Fixture::new();
        let mut client = Client::start(
            &fixture.profile(),
            fixture.workspace(),
            &Cancellation::default(),
            &Cancellation::default(),
            1,
        )
        .unwrap();
        let pid = client.pid();
        let outside = Directory::new();
        let base = outside.path().join("watched");
        std::fs::create_dir(&base).unwrap();
        let file = base.join("watch.txt");
        std::fs::write(&file, "content").unwrap();
        client.registrations.insert(
            "test".into(),
            vec![super::super::WatchPattern {
                pattern: crate::lsp::glob::Pattern::new("**/*.txt").unwrap(),
                base: base.clone(),
                kind: 7,
                absolute: false,
                literal: None,
            }],
        );
        client.poll_watches().unwrap();
        std::fs::remove_file(file).unwrap();
        client.poll_watches().unwrap();
        assert!(!client.sync_gap);
        std::fs::remove_dir(base).unwrap();
        client.poll_watches().unwrap();
        // A missing base is an empty scope, not an incomplete scan, so it
        // does not hold back events from the other watchers.
        assert!(!client.sync_gap);
        assert_eq!(client.pid(), pid);
        let document = client.reconcile(&fixture.sample(), "rust").unwrap();
        let response = client
            .query(
                "textDocument/definition",
                Value::object([
                    (
                        "textDocument",
                        Value::object([("uri", Value::string(&document.uri))]),
                    ),
                    (
                        "position",
                        document.model_position(1, 1, client.encoding).unwrap(),
                    ),
                ]),
                Duration::from_secs(2),
                &Cancellation::default(),
                &Cancellation::default(),
            )
            .unwrap();
        assert!(response.get("result").is_some());
        assert_eq!(client.pid(), pid);
        client.shutdown();
    }

    #[test]
    fn glob_segments_match_zero_or_more_directories() {
        assert!(glob_match("src/**/*.rs", "src/lib.rs"));
        assert!(glob_match("src/**/*.rs", "src/a/b/lib.rs"));
        assert!(!glob_match("src/*.rs", "src/a/lib.rs"));
    }

    #[test]
    fn literal_external_watches_observe_absence_without_scanning_the_parent() {
        let fixture = Fixture::new();
        let mut client = Client::start(
            &fixture.profile(),
            fixture.workspace(),
            &Cancellation::default(),
            &Cancellation::default(),
            1,
        )
        .unwrap();
        let outside = Directory::new();
        let parent = outside.path().join("not-created");
        let file = parent.join("settings.json");
        client.registrations.insert(
            "literal".into(),
            vec![super::super::WatchPattern {
                pattern: crate::lsp::glob::Pattern::new(&file.to_string_lossy()).unwrap(),
                base: parent.clone(),
                kind: 7,
                absolute: true,
                literal: Some(file.clone()),
            }],
        );
        client.poll_watches().unwrap();
        assert!(!client.sync_gap);
        assert!(!client.watched.contains_key(&file));
        std::fs::create_dir(&parent).unwrap();
        std::fs::write(&file, "configuration").unwrap();
        client.poll_watches().unwrap();
        assert!(!client.sync_gap && client.watched.contains_key(&file));
        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir(&parent).unwrap();
        client.poll_watches().unwrap();
        assert!(!client.sync_gap && !client.watched.contains_key(&file));
        client.shutdown();
    }
}
