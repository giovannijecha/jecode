//! Runtime provenance is a journal event, not part of the model conversation.

use crate::json::Value;
use std::sync::OnceLock;

pub(crate) fn activation() -> Value {
    static BUILD: OnceLock<Value> = OnceLock::new();
    event(
        env!("CARGO_PKG_VERSION"),
        BUILD.get_or_init(crate::identity::value).clone(),
        super::now(),
    )
}

fn event(version: &str, build: Value, at_unix_ms: u64) -> Value {
    Value::object([
        ("type", Value::string("runtime_identity")),
        ("at_unix_ms", Value::number(at_unix_ms)),
        ("jecode_version", Value::string(version)),
        ("build", build),
    ])
}

#[cfg(test)]
pub(super) fn fixture(version: &str, fingerprint: &str) -> Value {
    event(
        version,
        Value::object([
            ("os", Value::string("fixture-os")),
            ("arch", Value::string("fixture-arch")),
            ("profile", Value::string("fixture-profile")),
            ("fingerprint_algorithm", Value::string("fnv1a64")),
            ("executable_fingerprint", Value::string(fingerprint)),
        ]),
        1234,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        effort::Effort,
        export::Archive,
        redact::Redactor,
        sessions::{Document, Handle, Stage, Store},
        test_support::Directory,
    };
    use std::sync::{Arc, Mutex};

    #[test]
    fn two_runtime_activations_keep_the_first_build_and_saves_append_no_identity() {
        let home = Directory::new();
        let directory = Directory::new();
        let store = Store::new(home.path().to_path_buf(), directory.path()).unwrap();
        let mut document = Document::new(
            directory.path().to_path_buf(),
            "fixture/model".into(),
            Effort::High,
        )
        .unwrap();
        document.messages = [("system", "Fixture system"), ("user", "Fixture request")]
            .into_iter()
            .map(|(role, content)| {
                Value::object([
                    ("role", Value::string(role)),
                    ("content", Value::string(content)),
                ])
            })
            .collect();
        let archive = Archive {
            backend: "openrouter".into(),
            model: "fixture/model".into(),
            directory: directory.path().to_path_buf(),
            messages: Arc::new(Mutex::new(document.messages.clone())),
            redactor: Redactor::empty(),
            effort: "high".into(),
            events: Arc::new(Mutex::new(vec![])),
        };
        let id = document.id.clone();
        let handle = Handle::new(store.clone(), document, Redactor::empty());
        handle.flush().unwrap();
        handle
            .append_event(&archive, fixture("1.0.0", "aaaaaaaaaaaaaaaa"))
            .unwrap();
        handle
            .checkpoint(
                &archive,
                0,
                &crate::context::Context::default(),
                &crate::backend::Routing::Auto,
                true,
                Stage::Preserve,
            )
            .unwrap();
        drop(handle);

        let (resumed, _) = Handle::open(store.clone(), &id, Redactor::empty()).unwrap();
        let first = resumed.snapshot().events[0].clone();
        assert_eq!(
            first.get("jecode_version").and_then(Value::as_str),
            Some("1.0.0")
        );
        assert_eq!(
            first.get("after_message").and_then(Value::as_usize),
            Some(2)
        );
        assert_eq!(
            first.get("at_unix_ms").and_then(Value::as_usize),
            Some(1234)
        );
        assert_eq!(
            first
                .get("build")
                .and_then(|build| build.get("executable_fingerprint"))
                .and_then(Value::as_str),
            Some("aaaaaaaaaaaaaaaa")
        );
        let second_archive = Archive {
            events: Arc::new(Mutex::new(resumed.snapshot().events)),
            ..archive
        };
        resumed
            .append_event(&second_archive, fixture("2.0.0", "bbbbbbbbbbbbbbbb"))
            .unwrap();
        resumed
            .checkpoint(
                &second_archive,
                0,
                &crate::context::Context::default(),
                &crate::backend::Routing::Auto,
                true,
                Stage::Preserve,
            )
            .unwrap();
        let saved = resumed.snapshot();
        assert_eq!(saved.events.len(), 2);
        assert_eq!(saved.events[0], first);
        assert_eq!(
            saved.events[1]
                .get("jecode_version")
                .and_then(Value::as_str),
            Some("2.0.0")
        );
        assert_eq!(saved.records().len(), 1);
        drop(resumed);
        let (reopened, _) = Handle::open(store.clone(), &id, Redactor::empty()).unwrap();
        assert_eq!(reopened.snapshot().events, saved.events);
        let journal = std::fs::read_to_string(store.journal_path(&id)).unwrap();
        let appearances = journal.matches("runtime_identity").count();
        assert_eq!(appearances, 2);
    }
}
