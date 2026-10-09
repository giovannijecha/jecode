//! Attaching files and clipboard images to the composer draft. Imports run on
//! a worker thread so the composer stays responsive; each finished import
//! inserts its elements at the cursor of the draft that is current then.

use super::{App, Feedback, Kind};
use crate::{
    attachments::{Attachment, Pool, capture},
    cancel::Cancellation,
};
use std::{
    path::PathBuf,
    thread::{self, JoinHandle},
};

pub enum Source {
    Paths(Vec<PathBuf>),
    #[cfg_attr(test, allow(dead_code))]
    Clipboard,
    #[cfg(test)]
    Image(Result<Vec<u8>, String>),
}

pub struct Import {
    task: Option<JoinHandle<Vec<Result<Attachment, String>>>>,
    cancellation: Cancellation,
    session: Option<String>,
}

impl Import {
    fn start(pool: Pool, source: Source, session: Option<String>) -> Self {
        let cancellation = Cancellation::default();
        let token = cancellation.clone();
        let task = thread::spawn(move || match source {
            Source::Paths(paths) => paths
                .iter()
                .map(|path| pool.import_file(path, &token))
                .collect(),
            Source::Clipboard => vec![
                capture::clipboard_image(&token)
                    .and_then(|bytes| pool.import_bytes("clipboard.png", &bytes)),
            ],
            #[cfg(test)]
            Source::Image(bytes) => {
                vec![bytes.and_then(|bytes| pool.import_bytes("clipboard.png", &bytes))]
            }
        });
        Self {
            task: Some(task),
            cancellation,
            session,
        }
    }

    fn finished(&self) -> bool {
        self.task.as_ref().is_none_or(JoinHandle::is_finished)
    }

    fn finish(mut self) -> Vec<Result<Attachment, String>> {
        self.task
            .take()
            .map(|task| {
                task.join()
                    .unwrap_or_else(|_| vec![Err("Attachment import failed".into())])
            })
            .unwrap_or_default()
    }
}

impl Drop for Import {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            self.cancellation.cancel();
            let _ = task.join();
        }
    }
}

impl App {
    fn attachment_pool(&self) -> Option<Pool> {
        self.persistence
            .as_ref()
            .map(|handle| handle.store().attachments())
    }

    pub(super) fn attach_command(&mut self, arguments: &str) {
        match crate::attachments::paths::typed(arguments, &self.archive.directory) {
            Ok(paths) => self.attach(Source::Paths(paths)),
            Err(error) => self.warn(&error),
        }
    }

    pub(super) fn attach(&mut self, source: Source) {
        let Some(pool) = self.attachment_pool() else {
            self.warn("Attachments need session storage, which is unavailable.");
            return;
        };
        let text = match &source {
            Source::Paths(paths) if paths.len() == 1 => "Attaching 1 file…".to_owned(),
            Source::Paths(paths) => format!("Attaching {} files…", paths.len()),
            _ => "Reading the clipboard image…".to_owned(),
        };
        self.state.notify(Feedback::progress(Kind::Notice, text));
        let session = self.persistence.as_ref().map(|handle| handle.id());
        self.imports.push(Import::start(pool, source, session));
    }

    /// Inserts finished imports in the order they started.
    pub(super) fn poll_imports(&mut self) -> bool {
        let mut dirty = false;
        while self.imports.first().is_some_and(Import::finished) {
            let import = self.imports.remove(0);
            let session = import.session.clone();
            let results = import.finish();
            dirty = true;
            if session != self.persistence.as_ref().map(|handle| handle.id()) {
                self.warn("An attachment finished after the session changed; attach it again.");
                continue;
            }
            let mut attached = 0;
            let mut errors = Vec::new();
            for result in results {
                match result {
                    Ok(attachment) => {
                        self.insert_attachment(attachment);
                        attached += 1;
                    }
                    Err(error) => errors.push(error),
                }
            }
            if attached > 0 {
                self.edited();
                self.state.history.edited();
                self.persist_input(true);
            }
            if !errors.is_empty() {
                self.warn(&format!("{} Your draft was kept.", errors.join(" · ")));
            } else if attached > 0 {
                self.state.notify(Feedback::result(
                    Kind::Notice,
                    if attached == 1 {
                        "Attached 1 item".to_owned()
                    } else {
                        format!("Attached {attached} items")
                    },
                ));
            }
        }
        dirty
    }

    /// Removes pooled assets that no saved session refers to any more, once
    /// the grace period protects other running instances' fresh imports.
    pub(super) fn collect_attachments(&self) {
        self.capture_input();
        if let Some(handle) = &self.persistence {
            let store = handle.store();
            let mut live = handle.snapshot().input.attachment_ids();
            for message in self.archive.messages.lock().unwrap().iter() {
                live.extend(crate::attachments::references(message.encode().as_bytes()));
            }
            let none = std::collections::BTreeSet::new();
            let _ = store.attachments().collect(store.bucket(), &live, &none);
        }
    }

    fn insert_attachment(&mut self, attachment: Attachment) {
        let editor = &mut self.state.editor;
        editor.attach(attachment);
        if !editor.text[editor.cursor..].starts_with(char::is_whitespace) {
            editor.insert(" ");
        }
    }
}

#[cfg(test)]
mod tests;
