use crate::json::Value;
use crate::redact::Redactor;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
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
            let result = file
                .write_all(document.pretty().as_bytes())
                .and_then(|_| file.write_all(b"\n"))
                .and_then(|_| file.sync_all());
            drop(file);
            if let Err(error) = result {
                let _ = fs::remove_file(&path);
                return Err(format!("Could not write conversation export: {error}"));
            }
            return Ok(path);
        }
        Err("Could not reserve a unique export filename".into())
    }
}

fn timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests;
