use super::*;
use crate::session::Model;
use std::time::Duration;

fn listing() -> Vec<Listed> {
    vec![
        Listed {
            id: "s-0000000000000002-00000000-00000000".into(),
            model: Some(Model::Luna),
            title: "Read settings.rs, change retry_limit from 2 to 3, then run the tests".into(),
            turns: 1,
            workspace: Some(std::path::PathBuf::from("project-b")),
            directory: Some(std::path::PathBuf::from("project-b")),
            modified: UNIX_EPOCH + Duration::from_secs(1000),
        },
        Listed {
            id: "s-0000000000000001-00000000-00000000".into(),
            model: None,
            title: "Unreadable session".into(),
            turns: 0,
            workspace: None,
            directory: None,
            modified: UNIX_EPOCH,
        },
    ]
}

#[test]
fn cards_fit_narrow_terminals_and_do_not_expose_injected_controls() {
    let mut sessions = listing();
    sessions[0].title.push_str("\n\x1b[2J\u{202e} fake\trow 👩‍💻");
    sessions[0].workspace = Some("project\n\x1b[0m\u{2066}".into());
    let now = UNIX_EPOCH + Duration::from_secs(1120);
    for width in [1, 8, 24, 40, 80, 120] {
        let rows = rows(&sessions, width, now, false);
        for row in &rows {
            let output = row.paint(false);
            assert!(text::width(&output) <= width.saturating_sub(1).max(1));
            assert!(!output.chars().any(char::is_control));
            assert!(!output.contains(['\u{202e}', '\u{2066}']));
        }
        assert!(!rows.iter().any(|row| row.text.contains("s-000")));
    }
    let output: Vec<_> = rows(&listing(), 100, now, false)
        .into_iter()
        .map(|r| r.text)
        .collect();
    assert!(
        output
            .iter()
            .any(|row| row.contains("2m ago · 1 turn · gpt-5.6-luna"))
    );
    assert!(
        output
            .iter()
            .any(|row| row.starts_with(" 1. Read settings"))
    );
    assert!(
        output
            .iter()
            .any(|row| row.contains("Cannot resume; saved file kept."))
    );
    assert!(output.iter().any(|row| row.starts_with("    project-b")));
    let pipe = rows(&listing(), 80, now, true);
    assert!(pipe.iter().any(|row| row.text.contains(&listing()[0].id)));
}

#[test]
fn selection_uses_the_displayed_order_and_rejects_unreadable_or_invalid_choices() {
    let sessions = listing();
    let mut choice = Choice::default();
    assert_eq!(
        choice.key(Key::Text("1".into()), &sessions),
        Decision::Pending
    );
    assert_eq!(choice.key(Key::Enter, &sessions), Decision::Selected(0));
    for invalid in ["0", "2", "3", "100", "1wrong", "1234", "1\n2"] {
        let decision = choice.key(Key::Text(invalid.into()), &sessions);
        if decision == Decision::Pending {
            assert_eq!(choice.key(Key::Enter, &sessions), Decision::Invalid);
        } else {
            assert_eq!(decision, Decision::Invalid);
        }
        assert!(choice.input.is_empty());
    }
    choice.key(Key::Text("12".into()), &sessions);
    choice.key(Key::Backspace, &sessions);
    assert_eq!(choice.key(Key::Enter, &sessions), Decision::Selected(0));
    for key in [Key::Escape, Key::Interrupt, Key::Quit, Key::Enter] {
        assert_eq!(choice.key(key, &sessions), Decision::Cancel);
    }
}

#[test]
fn empty_titles_times_and_unicode_truncation_remain_explicit() {
    let now = UNIX_EPOCH + Duration::from_secs(1120);
    assert!(
        rows(&[], 80, now, false)
            .iter()
            .any(|r| r.text == "No saved sessions yet.")
    );
    let mut sessions = listing();
    sessions[0].title.clear();
    sessions[0].workspace = None;
    sessions[0].directory = None;
    assert!(
        rows(&sessions, 80, now, false)
            .iter()
            .any(|r| r.text.contains("Untitled conversation"))
    );
    assert!(
        rows(&sessions, 80, now, false)
            .iter()
            .any(|r| r.text.contains("Conversation only"))
    );
    assert_eq!(age(UNIX_EPOCH, now), "Activity time unknown");
    assert_eq!(age(now + Duration::from_secs(60), now), "Just now");
    assert_eq!(clipped("ab👩‍💻cdef", 5), "a…");
    assert_eq!(clipped("e\u{301}abcdef", 1), "…");
}
