use super::*;
use crate::attachments::{MARKER, Prompt};

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

#[test]
fn legacy_deltas_remove_sent_attachments_even_after_an_earlier_resume() {
    for previously_resumed in [false, true] {
        let (_directory, _home, store, mut doc) = fixture();
        let image = store
            .attachments()
            .import_bytes("shot.png", b"image")
            .unwrap();
        doc.input.draft = Draft::from_prompt(Prompt::new(String::new(), vec![image]));
        let lease = store.create(&doc.id).unwrap();
        lease.save(&doc).unwrap();
        doc.input.draft = Draft::default();
        lease.save(&doc).unwrap();
        drop(lease);
        let path = store.journal_path(&doc.id);
        let mut records: Vec<_> = fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|line| crate::json::parse(line).unwrap())
            .collect();
        let Value::Object(record) = &mut records[1] else {
            unreachable!()
        };
        let Value::Object(state) = record.get_mut("state").unwrap() else {
            unreachable!()
        };
        let Value::Object(input) = state.get_mut("input").unwrap() else {
            unreachable!()
        };
        let Value::Object(draft) = input.get_mut("draft").unwrap() else {
            unreachable!()
        };
        // Older writers omitted this field when the last attachment was sent or removed.
        draft.remove("attachments");
        if previously_resumed {
            let mut resumed = records[1].clone();
            let Value::Object(fields) = &mut resumed else {
                unreachable!()
            };
            fields.insert("sequence".into(), Value::number(2));
            fields.insert(
                "state".into(),
                Value::object([(
                    "input",
                    Value::object([(
                        "draft",
                        Value::object([
                            ("text", Value::string(MARKER.to_string())),
                            ("cursor", Value::number(MARKER.len_utf8())),
                        ]),
                    )]),
                )]),
            );
            records.push(resumed);
        }
        let original = records
            .iter()
            .map(|record| format!("{}\n", record.encode()))
            .collect::<String>();
        fs::write(&path, &original).unwrap();
        assert_eq!(
            store.fixture_load(&doc.id).unwrap().input.draft,
            Draft::default()
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        let (handle, _) = Handle::open(store.clone(), &doc.id, Redactor::empty()).unwrap();
        assert_eq!(handle.snapshot().input.draft, Draft::default());
        drop(handle);
        assert_eq!(
            store.fixture_load(&doc.id).unwrap().input.draft,
            Draft::default()
        );
    }
}
