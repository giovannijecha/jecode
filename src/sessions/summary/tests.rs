use super::*;
use crate::{effort::Effort, redact::Redactor, sessions::Handle, test_support::Directory};

fn fixture() -> (Directory, Directory, Store, Document) {
    let home = Directory::new();
    let directory = Directory::new();
    let store = Store::new(home.path().to_path_buf(), directory.path()).unwrap();
    let mut document = Document::new(
        directory.path().to_path_buf(),
        "fixture/model".into(),
        Effort::Default,
    )
    .unwrap();
    document.messages = vec![Value::object([
        ("role", Value::string("system")),
        ("content", Value::string("Fixture")),
    ])];
    document.input.draft.text = "A draft-only session 🦀".into();
    document.input.draft.cursor = document.input.draft.text.len();
    (home, directory, store, document)
}

#[test]
fn summaries_track_the_committed_journal_and_bound_large_titles() {
    let (_home, _directory, store, mut document) = fixture();
    let lease = store.create(&document.id).unwrap();
    lease.save(&document).unwrap();
    let first = stamp(&store, &document.id).unwrap();
    assert_eq!(
        load(&store, &document.id, &first).unwrap().title,
        document.input.draft.text
    );
    assert!(fs::metadata(path(&store, &document.id)).unwrap().len() < LIMIT);
    document.messages.push(Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("🦀 ".repeat(256 * 1024))),
    ]));
    document.model = "fixture/changed".into();
    lease.save(&document).unwrap();
    assert!(load(&store, &document.id, &first).is_none());
    let summary = load(&store, &document.id, &stamp(&store, &document.id).unwrap()).unwrap();
    assert_eq!(summary.model, "fixture/changed");
    assert!(summary.title.chars().count() <= 100);
    assert_eq!(store.list().unwrap().sessions.len(), 1);
}

#[test]
fn missing_corrupt_or_stale_caches_fall_back_to_the_journal() {
    let (_home, _directory, store, document) = fixture();
    let lease = store.create(&document.id).unwrap();
    lease.save(&document).unwrap();
    let cache = path(&store, &document.id);
    let valid = fs::read_to_string(&cache).unwrap();
    fs::remove_file(&cache).unwrap();
    assert_eq!(store.list().unwrap().sessions[0].title, document.title());
    assert!(fs::metadata(&cache).is_ok());
    for text in [
        "{broken".into(),
        valid.replace("\"journal_bytes\":", "\"old_bytes\":"),
        "x".repeat(LIMIT as usize + 1),
    ] {
        fs::write(&cache, text).unwrap();
        let listing = store.list().unwrap();
        assert!(listing.warnings.is_empty());
        assert_eq!(listing.sessions[0].title, document.title());
        assert!(load(&store, &document.id, &stamp(&store, &document.id).unwrap()).is_some());
    }
}

#[test]
fn cache_write_failure_does_not_fail_a_committed_save_or_hide_the_session() {
    let (_home, _directory, store, document) = fixture();
    let lease = store.create(&document.id).unwrap();
    fs::create_dir(path(&store, &document.id)).unwrap();
    lease.save(&document).unwrap();
    assert!(store.fixture_load(&document.id).unwrap().input == document.input);
    assert_eq!(store.list().unwrap().sessions[0].title, document.title());
    drop(lease);
    assert!(Handle::open(store, &document.id, Redactor::empty()).is_ok());
}

#[test]
fn listing_uses_a_valid_cache_but_resume_always_validates_the_journal() {
    let (_home, _directory, store, document) = fixture();
    let lease = store.create(&document.id).unwrap();
    lease.save(&document).unwrap();
    drop(lease);
    let journal = store.journal_path(&document.id);
    let bytes = fs::read(&journal).unwrap();
    fs::write(&journal, vec![b'x'; bytes.len()]).unwrap();
    // An artificial matching cache proves listing does not parse the journal.
    // Build its fingerprint from the file rather than relying on mtime round-trips.
    save(&store, &document, &stamp(&store, &document.id).unwrap()).unwrap();
    assert_eq!(store.list().unwrap().sessions[0].title, document.title());
    assert!(Handle::open(store.clone(), &document.id, Redactor::empty()).is_err());
    fs::write(journal, bytes).unwrap();
    assert!(Handle::open(store, &document.id, Redactor::empty()).is_ok());
}
