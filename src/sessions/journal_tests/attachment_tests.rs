use super::*;
use crate::attachments::Prompt;

#[test]
fn clearing_the_last_draft_attachment_survives_journal_reload() {
    for text in ["", "Next request"] {
        let (_directory, _home, store, mut doc) = fixture();
        let image = store
            .attachments()
            .import_bytes("shot.png", b"image")
            .unwrap();
        doc.input.draft = Draft::from_prompt(Prompt::new("Compare".into(), vec![image]));
        let lease = store.create(&doc.id).unwrap();
        lease.save(&doc).unwrap();
        doc.input.draft = Draft::from_prompt(Prompt::plain(text));
        lease.save(&doc).unwrap();
        drop(lease);
        assert_eq!(
            store.fixture_load(&doc.id).unwrap().input.draft,
            doc.input.draft
        );
    }
}

#[test]
fn unsent_draft_attachments_survive_text_and_cursor_deltas() {
    let (_directory, _home, store, mut doc) = fixture();
    let image = store
        .attachments()
        .import_bytes("shot.png", b"image")
        .unwrap();
    doc.input.draft = Draft::from_prompt(Prompt::new(String::new(), vec![image.clone()]));
    let lease = store.create(&doc.id).unwrap();
    lease.save(&doc).unwrap();
    doc.input.draft = Draft::from_prompt(Prompt::new("Still editing".into(), vec![image]));
    lease.save(&doc).unwrap();
    doc.input.draft.cursor = 0;
    lease.save(&doc).unwrap();
    drop(lease);
    assert_eq!(
        store.fixture_load(&doc.id).unwrap().input.draft,
        doc.input.draft
    );
}
