//! The `@` browser: one workspace folder at a time, listed and searched off the
//! interface thread through the same walk, ignore rules and boundary as `list`.
//!
//! Each folder starts one background walk. It sends the folder's children first, then
//! its subtree in batches for the filter, and stops when cancelled. A walk belongs to one
//! folder generation; results of any other generation are discarded, and dropping the
//! browser cancels its walk. Filtering never walks: it ranks what the walk delivered.

use super::Origin;
use crate::{cancel::Cancellation, json::Value, tools::discovery::walk::Walk, tui::editor::Editor};
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

/// Entries one subtree search ranks at most; the aside says when a search stopped there.
pub const SEARCHED: usize = 20_000;
const BATCH: usize = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Directory,
    File,
    Link,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Workspace-relative, with `/` separators.
    pub path: String,
    pub kind: Kind,
}

impl Entry {
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

enum Message {
    Children(u64, Result<Vec<Entry>, String>),
    Subtree(u64, Vec<Entry>, bool),
}

/// A running walk; dropping it asks the thread to stop without waiting for it.
struct Search {
    receiver: Receiver<Message>,
    cancellation: Cancellation,
}

impl Drop for Search {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

/// A folder left for a subfolder, restored when Backspace returns to it.
struct Level {
    directory: String,
    filter: Editor,
    focus: Option<String>,
}

pub struct Browser {
    pub origin: Origin,
    root: PathBuf,
    /// The conversation the browser opened in; another one makes its results stale.
    pub session: Option<String>,
    /// The folder shown, workspace-relative; empty at the root.
    pub directory: String,
    /// The folder's children, directories first; `None` while they are read.
    pub children: Option<Vec<Entry>>,
    /// The folder's subtree as far as the walk has gone.
    pub subtree: Vec<Entry>,
    pub searching: bool,
    /// The walk stopped at `SEARCHED` entries.
    pub capped: bool,
    pub error: Option<String>,
    /// The entry to focus once it is listed.
    pub focus: Option<String>,
    trail: Vec<Level>,
    generation: u64,
    search: Option<Search>,
    /// The entries behind the selector's rows, in the same order.
    pub rows: Vec<Entry>,
}

impl Browser {
    pub fn new(origin: Origin, root: PathBuf, session: Option<String>) -> Self {
        let mut browser = Self {
            origin,
            root,
            session,
            directory: String::new(),
            children: None,
            subtree: Vec::new(),
            searching: false,
            capped: false,
            error: None,
            focus: None,
            trail: Vec::new(),
            generation: 0,
            search: None,
            rows: Vec::new(),
        };
        browser.load();
        browser
    }

    /// The workspace folder's name, then the folder shown.
    pub fn location(&self) -> String {
        let name = self
            .root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "workspace".into());
        if self.directory.is_empty() {
            name
        } else {
            format!("{name}/{}", self.directory)
        }
    }

    /// Starts reading the current folder, discarding whatever the last walk had found.
    pub fn load(&mut self) {
        self.generation += 1;
        self.children = None;
        self.subtree.clear();
        self.searching = true;
        self.capped = false;
        self.error = None;
        self.search = Some(start(
            self.root.clone(),
            self.directory.clone(),
            self.generation,
        ));
    }

    /// Whether a walk still has results to deliver.
    pub fn pending(&self) -> bool {
        self.search.is_some()
    }

    /// Takes what the walk delivered; true when the rows changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        loop {
            let Some(search) = &self.search else {
                return changed;
            };
            let message = match search.receiver.try_recv() {
                Ok(message) => message,
                Err(TryRecvError::Empty) => return changed,
                Err(TryRecvError::Disconnected) => {
                    self.search = None;
                    self.searching = false;
                    return true;
                }
            };
            match message {
                Message::Children(generation, _) | Message::Subtree(generation, ..)
                    if generation != self.generation => {}
                Message::Children(_, Ok(children)) => {
                    self.children = Some(children);
                    changed = true;
                }
                Message::Children(_, Err(error)) => {
                    self.children = Some(Vec::new());
                    self.error = Some(error);
                    self.searching = false;
                    self.search = None;
                    changed = true;
                }
                Message::Subtree(_, entries, done) => {
                    self.subtree.extend(entries);
                    if done {
                        self.capped = self.subtree.len() >= SEARCHED;
                        self.searching = false;
                        self.search = None;
                    }
                    changed = true;
                }
            }
        }
    }

    /// Opens a subfolder, remembering this folder's filter and focus for the way back.
    pub fn enter(&mut self, directory: &str, filter: &Editor, focus: Option<String>) {
        self.trail.push(Level {
            directory: std::mem::replace(&mut self.directory, directory.into()),
            filter: filter.clone(),
            focus,
        });
        self.focus = None;
        self.load();
    }

    /// The parent folder and the filter to restore there; `None` at the root. Returning
    /// where the user came from restores that folder's filter and focus; otherwise the
    /// folder just left takes the focus.
    pub fn leave(&mut self) -> Option<Editor> {
        if self.directory.is_empty() {
            return None;
        }
        let parent = self
            .directory
            .rsplit_once('/')
            .map_or(String::new(), |(parent, _)| parent.into());
        let left = std::mem::replace(&mut self.directory, parent);
        let filter = match self.trail.pop() {
            Some(level) if level.directory == self.directory => {
                self.focus = level.focus;
                level.filter
            }
            _ => {
                self.trail.clear();
                self.focus = Some(left);
                Editor::default()
            }
        };
        self.load();
        Some(filter)
    }

    /// A typed path that names an existing entry, hidden and ignored ones included, as
    /// an explicit path does for the file tools.
    pub fn explicit(&self, filter: &str) -> Option<Entry> {
        let typed = super::syntax::typed_path(filter);
        let typed = typed.trim_end_matches('/');
        if typed.is_empty() || typed == "." {
            return None;
        }
        let relative = if Path::new(typed).is_absolute() || self.directory.is_empty() {
            typed.to_string()
        } else {
            format!("{}/{typed}", self.directory)
        };
        let source = self.root.join(&relative);
        let parent = source.parent()?;
        // An exact path must not provide a way to browse through a linked folder. Only
        // folders below the workspace count: an absolute path may reach the workspace
        // itself through a linked prefix, such as `/tmp` for `/private/tmp` on macOS.
        let root = self.root.as_path();
        if parent
            .ancestors()
            .take_while(|path| {
                *path != root && std::fs::canonicalize(path).ok().as_deref() != Some(root)
            })
            .any(|path| {
                std::fs::symlink_metadata(path)
                    .is_ok_and(|metadata| metadata.file_type().is_symlink())
            })
        {
            return None;
        }
        let cancellation = Cancellation::default();
        let arguments = Value::object([("path", Value::string(&relative))]);
        let walk = Walk::new(&self.root, &arguments, false, true, &cancellation).ok()?;
        if walk.scope == "." {
            return None;
        }
        // Walk validates the resolved target and workspace boundary. Its scope holds
        // the stored names, which a case-insensitive file system may spell unlike the
        // typed path; a link keeps its original final name, its own reference and kind.
        let parent = std::fs::canonicalize(parent).ok()?;
        let source = parent.join(source.file_name()?);
        let kind = kind(&source);
        let path = if kind == Kind::Link {
            let path = source.strip_prefix(&self.root).ok()?.to_str()?.to_owned();
            #[cfg(windows)]
            let path = path.replace('\\', "/");
            path
        } else {
            walk.scope
        };
        Some(Entry { kind, path })
    }

    /// Whether `path` still exists, without following a final link.
    pub fn exists(&self, path: &str) -> bool {
        std::fs::symlink_metadata(self.root.join(path)).is_ok()
    }
}

fn kind(path: &Path) -> Kind {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Kind::Link,
        Ok(metadata) if metadata.is_dir() => Kind::Directory,
        Ok(metadata) if metadata.is_file() => Kind::File,
        _ => Kind::Other,
    }
}

fn start(root: PathBuf, directory: String, generation: u64) -> Search {
    let (sender, receiver) = mpsc::channel();
    let cancellation = Cancellation::default();
    let token = cancellation.clone();
    thread::spawn(move || {
        let children = children(&root, &directory, &token);
        let failed = children.is_err();
        if sender
            .send(Message::Children(generation, children))
            .is_err()
            || failed
        {
            return;
        }
        let path = if directory.is_empty() {
            "."
        } else {
            &directory
        };
        let arguments = Value::object([("path", Value::string(path))]);
        let Ok(mut walk) = Walk::new(&root, &arguments, true, false, &token) else {
            let _ = sender.send(Message::Subtree(generation, Vec::new(), true));
            return;
        };
        let mut batch = Vec::new();
        let mut total = 0;
        loop {
            match walk.next() {
                Ok(Some(entry)) if total < SEARCHED => {
                    total += 1;
                    batch.push(from_walk(&entry.relative, entry.kind));
                    if batch.len() == BATCH {
                        let batch = std::mem::take(&mut batch);
                        if sender
                            .send(Message::Subtree(generation, batch, false))
                            .is_err()
                        {
                            return;
                        }
                    }
                }
                Ok(_) => break,
                // Cancelled: nobody is waiting for the rest.
                Err(_) if token.requested() => return,
                Err(_) => break,
            }
        }
        let _ = sender.send(Message::Subtree(generation, batch, true));
    });
    Search {
        receiver,
        cancellation,
    }
}

fn from_walk(relative: &str, kind: &str) -> Entry {
    Entry {
        path: relative.into(),
        kind: match kind {
            "directory" => Kind::Directory,
            "file" => Kind::File,
            "symlink" => Kind::Link,
            _ => Kind::Other,
        },
    }
}

/// The folder's own entries, directories first, each group by name; a folder that
/// cannot be read is an error rather than an empty list.
fn children(root: &Path, directory: &str, token: &Cancellation) -> Result<Vec<Entry>, String> {
    let path = if directory.is_empty() { "." } else { directory };
    let arguments = Value::object([("path", Value::string(path))]);
    let mut walk = Walk::new(root, &arguments, false, false, token)?;
    let mut entries = Vec::new();
    while let Some(entry) = walk.next()? {
        entries.push(from_walk(&entry.relative, entry.kind));
    }
    if entries.is_empty()
        && let Some(error) = walk.errors.first()
    {
        let reason = error
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unreadable");
        return Err(format!("Could not open {path}: {reason}"));
    }
    entries.sort_by_cached_key(|entry| {
        (
            entry.kind != Kind::Directory,
            entry.name().to_lowercase(),
            entry.name().to_string(),
        )
    });
    Ok(entries)
}
