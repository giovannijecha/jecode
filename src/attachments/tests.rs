use super::*;
use crate::test_support::Directory;
use std::collections::BTreeSet;
use std::fs;

/// A valid PNG header with the requested dimensions.
pub fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    bytes.extend(width.to_be_bytes());
    bytes.extend(height.to_be_bytes());
    bytes.extend([8, 6, 0, 0, 0]);
    bytes
}

pub fn attachment(id: &str, name: &str, media: &str) -> Attachment {
    Attachment {
        id: id.into(),
        name: name.into(),
        media: media.into(),
        size: 4,
        width: None,
        height: None,
        pages: None,
    }
}

#[test]
fn labels_are_derived_and_lookalikes_stay_text() {
    let image = attachment("att-1-1-1", "shot.png", "image/png");
    let report = attachment("att-1-1-2", "report.pdf", "application/pdf");
    let prompt = Prompt::new(
        format!("see {MARKER} and {MARKER} [3# Image]"),
        vec![image.clone(), report],
    );
    assert_eq!(
        prompt.display(),
        "see [1# Image] and [2# File: report.pdf] [3# Image]"
    );
    assert_eq!(prompt.attachments.len(), 2);
    // Typed or pasted markers never become attachments.
    let typed = Prompt::plain(format!("[1# Image] {MARKER}"));
    assert_eq!(typed.text, "[1# Image] ");
    assert!(typed.attachments.is_empty());
    // A mismatched count is repaired instead of misassigning data.
    let repaired = Prompt::new(format!("a{MARKER}{MARKER}"), vec![image]);
    assert_eq!(repaired.text, format!("a {MARKER}"));
    assert!(!Prompt::new(MARKER.to_string(), vec![attachment("att-1-1-3", "x", "x")]).is_empty());
}

#[test]
fn prompts_round_trip_and_accept_legacy_strings() {
    let prompt = Prompt::new(
        format!("{MARKER} explain"),
        vec![Attachment {
            width: Some(3),
            height: Some(2),
            ..attachment("att-5-6-7", "a.png", "image/png")
        }],
    );
    assert_eq!(Prompt::parse(&prompt.value()).unwrap(), prompt);
    assert_eq!(
        Prompt::parse(&crate::json::Value::string("old")).unwrap(),
        Prompt::plain("old")
    );
    let message = prompt.message();
    assert_eq!(
        message.get("content").and_then(crate::json::Value::as_str),
        Some("[1# Image] explain")
    );
    assert_eq!(of_message(&message), prompt.attachments);
    let invalid = crate::json::parse(
        r#"{"text":"x","attachments":[{"id":"../x","name":"a","media":"b","size":1}]}"#,
    )
    .unwrap();
    assert!(Prompt::parse(&invalid).is_err());
}

#[test]
fn pool_keeps_exact_bytes_of_external_files() {
    let pool_root = Directory::new();
    let outside = Directory::new();
    let source = outside.path().join("CON.bin");
    let bytes = (0..=255u8).cycle().take(300_000).collect::<Vec<_>>();
    fs::write(&source, &bytes).unwrap();
    let pool = Pool::new(pool_root.path().join("attachments"));
    let attachment = pool
        .import_file(&source, &crate::cancel::Cancellation::default())
        .unwrap();
    // Later changes to the source do not reach the stored copy.
    fs::write(&source, b"changed").unwrap();
    let stored = pool.load(&attachment.id).unwrap();
    assert_eq!(fs::read(&stored.path).unwrap(), bytes);
    assert_eq!(stored.attachment, attachment);
    assert_eq!(attachment.name, "CON.bin");
    assert_eq!(stored.path.file_name().unwrap(), "_CON.bin");
    assert_eq!(attachment.media, "application/octet-stream");
    assert_eq!(attachment.size, 300_000);
    assert!(stored.source.is_some());
    assert!(pool.load("att-../../x").is_err());
    assert!(
        pool.import_file(outside.path(), &crate::cancel::Cancellation::default())
            .is_err()
    );
}

#[test]
fn small_supported_images_are_sent_unchanged() {
    let root = Directory::new();
    let pool = Pool::new(root.path().to_path_buf());
    let data = png(4, 3);
    let attachment = pool.import_bytes("clip.png", &data).unwrap();
    assert_eq!(attachment.kind(), Kind::Image);
    assert_eq!((attachment.width, attachment.height), (Some(4), Some(3)));
    assert_eq!(
        pool.view(&attachment).unwrap(),
        Some(("image/png".into(), data))
    );
}

#[test]
fn collection_removes_only_unreferenced_owned_assets() {
    let root = Directory::new();
    let sessions = root.path().join("sessions");
    fs::create_dir(&sessions).unwrap();
    let pool = Pool::new(root.path().join("attachments"));
    let kept = pool.import_bytes("kept.txt", b"a").unwrap();
    let live = pool.import_bytes("live.txt", b"b").unwrap();
    let released = pool.import_bytes("released.txt", b"c").unwrap();
    let recent = pool.import_bytes("recent.txt", b"d").unwrap();
    fs::write(
        sessions.join("1-2-3.jsonl"),
        format!("{{\"id\":\"{}\"}}", kept.id),
    )
    .unwrap();
    fs::write(pool.directory().join("notes.txt"), b"not owned").unwrap();
    // An open session holds a byte-range lock on its lease file.
    let lease = fs::File::create(sessions.join("1-2-3.lock")).unwrap();
    lease.lock().unwrap();
    let removed = pool
        .collect(
            &sessions,
            &BTreeSet::from([live.id.clone()]),
            &BTreeSet::from([released.id.clone(), kept.id.clone()]),
        )
        .unwrap();
    assert_eq!(removed, 1);
    assert!(pool.load(&kept.id).is_ok());
    assert!(pool.load(&live.id).is_ok());
    // Within the grace period an unreferenced import may belong to another
    // process's unsaved draft.
    assert!(pool.load(&recent.id).is_ok());
    assert!(pool.load(&released.id).is_err());
    assert!(!pool.directory().join(&released.id).exists());
    assert!(pool.directory().join("notes.txt").exists());
}

#[test]
fn finds_references_in_serialized_sessions() {
    let found = pool::references(b"x att-1-2-3\" att-4-5-6 att- att-x");
    assert_eq!(
        found,
        BTreeSet::from(["att-1-2-3".to_string(), "att-4-5-6".to_string()])
    );
}
