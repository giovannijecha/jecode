use crate::json::Value;
use crate::redact::Redactor;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

pub type Messages = Arc<Mutex<Vec<Value>>>;
static NEXT_EXPORT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub struct Archive {
    pub model: String,
    pub directory: PathBuf,
    pub messages: Messages,
    pub redactor: Redactor,
    pub effort: String,
    pub events: Messages,
    /// Where attached files live; exports copy the ones messages refer to.
    pub attachments: Option<crate::attachments::Pool>,
}

impl Archive {
    #[cfg(test)]
    pub fn document(&self) -> Value {
        self.document_at(timestamp())
    }

    fn document_at(&self, timestamp: u128) -> Value {
        self.redactor.value(&Value::object([
            ("format", Value::string("jecode.conversation")),
            ("format_version", Value::number(1)),
            ("jecode_version", Value::string(env!("CARGO_PKG_VERSION"))),
            ("exported_at_unix_ms", Value::number(timestamp)),
            ("model", Value::string(&self.model)),
            ("effort", Value::string(&self.effort)),
            ("events", Value::Array(self.events.lock().unwrap().clone())),
            (
                "working_directory",
                Value::string(self.directory.to_string_lossy()),
            ),
            (
                "messages",
                Value::Array(self.messages.lock().unwrap().clone()),
            ),
        ]))
    }

    /// Copies every attachment the conversation refers to beside `path` and
    /// lists them with paths relative to the export.
    fn bundle(&self, path: &Path) -> Result<Option<Value>, String> {
        let messages = self.messages.lock().unwrap().clone();
        let mut seen = std::collections::BTreeSet::new();
        let mut attachments: Vec<_> = messages
            .iter()
            .flat_map(crate::attachments::of_message)
            .chain(messages.iter().filter_map(read_attachment))
            .filter(|attachment| seen.insert(attachment.id.clone()))
            .collect();
        let annotation_ids: Vec<_> = messages
            .iter()
            .flat_map(crate::attachments::annotations::references)
            .filter(|id| seen.insert(id.clone()))
            .collect();
        if attachments.is_empty() && annotation_ids.is_empty() {
            return Ok(None);
        }
        let pool = self
            .attachments
            .as_ref()
            .ok_or("Attachment storage is unavailable; the export was not written")?;
        for id in annotation_ids {
            attachments.push(pool.load(&id)?.attachment);
        }
        let directory = bundle_directory(path);
        let folder = directory
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mut files = Vec::new();
        for attachment in &attachments {
            let copied = pool.copy_to(attachment, &directory).inspect_err(|_| {
                let _ = fs::remove_dir_all(&directory);
            })?;
            let mut entry = attachment.value();
            if let Value::Object(fields) = &mut entry {
                let name = copied.file_name().unwrap_or_default().to_string_lossy();
                fields.insert(
                    "path".into(),
                    Value::string(format!("{folder}/{}/{name}", attachment.id)),
                );
            }
            files.push(entry);
        }
        Ok(Some(Value::Array(files)))
    }

    pub fn save(&self) -> Result<PathBuf, String> {
        let timestamp = timestamp();
        let document = self.document_at(timestamp);
        for _ in 0..100 {
            let counter = NEXT_EXPORT.fetch_add(1, Ordering::Relaxed);
            let path = self.directory.join(format!(
                "JECODE-SESSION-{timestamp}-{}-{counter}.json",
                std::process::id()
            ));
            let file = OpenOptions::new().write(true).create_new(true).open(&path);
            let mut file = match file {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("Could not create conversation export: {error}")),
            };
            let bundle = match self.bundle(&path) {
                Ok(bundle) => bundle,
                Err(error) => {
                    drop(file);
                    let _ = fs::remove_file(&path);
                    return Err(error);
                }
            };
            let mut document = document;
            if let (Some(files), Value::Object(fields)) = (bundle, &mut document) {
                fields.insert("attachments".into(), files);
            }
            let result = file
                .write_all(document.pretty().as_bytes())
                .and_then(|_| file.write_all(b"\n"))
                .and_then(|_| file.sync_all());
            drop(file);
            if let Err(error) = result {
                let _ = fs::remove_file(&path);
                let _ = fs::remove_dir_all(bundle_directory(&path));
                return Err(format!("Could not write conversation export: {error}"));
            }
            return Ok(path);
        }
        Err("Could not reserve a unique export filename".into())
    }
}

/// A successful `read attachment:...` result names an asset even when this
/// conversation did not import it as a user attachment.
fn read_attachment(message: &Value) -> Option<crate::attachments::Attachment> {
    if message.get("role").and_then(Value::as_str) != Some("tool") {
        return None;
    }
    let result = crate::json::parse(message.get("content")?.as_str()?).ok()?;
    let attachment = crate::attachments::Attachment::parse(result.get("attachment")?).ok()?;
    (result.get("reference")?.as_str()? == attachment.reference()).then_some(attachment)
}

/// The folder beside an export that holds its attachments.
pub fn bundle_directory(path: &Path) -> PathBuf {
    path.with_extension("attachments")
}

fn timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests;
