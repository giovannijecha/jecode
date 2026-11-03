use super::Client;
use crate::json::Value;
use crate::lsp::documents::path_to_uri;
use std::collections::HashMap;
use std::io::Read;

impl Client {
    pub(super) fn poll_watches(&mut self) -> Result<(), String> {
        let patterns = self.registrations.values().flatten().collect::<Vec<_>>();
        if patterns.is_empty()
            && self.observed_extensions.is_empty()
            && self.observed_markers.is_empty()
        {
            return Ok(());
        }
        let mut stack = vec![self.root.clone()];
        for watch in &patterns {
            if watch.literal.is_none()
                && !watch.base.starts_with(&self.root)
                && !stack.contains(&watch.base)
            {
                stack.push(watch.base.clone());
            }
        }
        let mut found = HashMap::new();
        let mut count = 0;
        let mut bytes = 0_u64;
        for path in patterns.iter().filter_map(|watch| watch.literal.as_ref()) {
            count += 1;
            match hash_path(path, &mut bytes, true) {
                Ok(Some(hash)) => {
                    found.insert(path.clone(), hash);
                }
                Ok(None) => {}
                Err(()) => {
                    self.sync_gap = true;
                    return Ok(());
                }
            }
        }
        while let Some(folder) = stack.pop() {
            let entries = match std::fs::read_dir(folder) {
                Ok(entries) => entries,
                Err(_) => {
                    self.sync_gap = true;
                    return Ok(());
                }
            };
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(_) => {
                        self.sync_gap = true;
                        return Ok(());
                    }
                };
                count += 1;
                if count > 10_000 {
                    self.sync_gap = true;
                    return Ok(());
                }
                let path = entry.path();
                let kind = match entry.file_type() {
                    Ok(kind) => kind,
                    Err(_) => {
                        self.sync_gap = true;
                        return Ok(());
                    }
                };
                if kind.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !kind.is_file() {
                    continue;
                }
                if !self.generic_file(&path)
                    && !patterns.iter().any(|pattern| pattern.matches(&path))
                {
                    continue;
                }
                match hash_path(&path, &mut bytes, false) {
                    Ok(Some(hash)) => {
                        found.insert(path, hash);
                    }
                    Ok(None) => {}
                    Err(()) => {
                        self.sync_gap = true;
                        return Ok(());
                    }
                }
            }
        }
        let changes = found
            .iter()
            .filter_map(|(path, hash)| match self.watched.get(path) {
                None => Some((path.clone(), 1)),
                Some(previous) if previous != hash => Some((path.clone(), 2)),
                _ => None,
            })
            .chain(
                self.watched
                    .keys()
                    .filter(|path| !found.contains_key(*path))
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
        if !self.watch_initialized {
            self.watch_initialized = true;
            return Ok(());
        }
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

    fn generic_file(&self, path: &std::path::Path) -> bool {
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
                .any(|known| relative == std::path::Path::new(known))
        });
        associated || marker
    }
}

fn hash_path(path: &std::path::Path, bytes: &mut u64, directory: bool) -> Result<Option<u64>, ()> {
    use std::io::ErrorKind;
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    };
    if metadata.is_dir() {
        return Ok(directory.then(|| {
            metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |time| time.as_nanos() as u64)
        }));
    }
    if !metadata.is_file() {
        return Ok(None);
    }
    let remaining = (32 * 1024 * 1024_u64).saturating_sub(*bytes);
    if metadata.len() > remaining {
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
    *bytes += data.len() as u64;
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in data {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    Ok(Some(hash))
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
        assert!(client.sync_gap);
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
