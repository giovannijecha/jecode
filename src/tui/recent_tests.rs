use super::delete_tests::{app, confirm, seed};
use super::*;
use crate::test_support::Directory;

#[test]
fn startup_offers_up_to_three_other_conversations_from_this_folder() {
    let home = Directory::new();
    let directory = Directory::new();
    assert!(app(&home, &directory).state.opening.recent.is_empty());
    for title in ["One", "Two", "Three", "Four"] {
        seed(&mut app(&home, &directory), title);
    }
    let fresh = app(&home, &directory);
    let titles: std::collections::BTreeSet<_> = fresh
        .state
        .opening
        .recent
        .iter()
        .map(|recent| recent.title.as_str())
        .collect();
    assert_eq!(titles.len(), 3);
    assert!(titles.is_subset(&["One", "Two", "Three", "Four"].into()));
    assert!(
        fresh
            .state
            .opening
            .recent
            .iter()
            .all(|recent| recent.age == "just now")
    );
    // The space replaces the former startup notice.
    assert!(fresh.state.notice.is_none());
}

#[test]
fn deleting_a_recent_conversation_removes_it_from_the_startup_space() {
    let home = Directory::new();
    let directory = Directory::new();
    for title in ["One", "Two", "Three", "Four"] {
        seed(&mut app(&home, &directory), title);
    }
    let mut fresh = app(&home, &directory);
    let titles = |app: &App| -> Vec<String> {
        let recent = app.state.opening.recent.iter();
        recent.map(|recent| recent.title.clone()).collect()
    };
    let offered = titles(&fresh);
    assert_eq!(offered.len(), 3);
    fresh.dispatch("/resume".into()).unwrap();
    let id = fresh
        .saved_sessions
        .iter()
        .find(|entry| entry.title == offered[0])
        .unwrap()
        .id
        .clone();
    confirm(&mut fresh, &id);
    fresh.finish_delete(true).unwrap();
    // The fourth conversation takes the deleted one's place.
    let after = titles(&fresh);
    assert_eq!(after.len(), 3);
    assert!(!after.contains(&offered[0]));
    for _ in 0..3 {
        let id = fresh.saved_sessions[0].id.clone();
        confirm(&mut fresh, &id);
        fresh.finish_delete(true).unwrap();
    }
    // With nothing left to resume, the space shows the key row again.
    assert!(fresh.state.opening.recent.is_empty());
    let rows = super::view::opening::rows(&fresh.state, 80);
    let plain: Vec<_> = rows.iter().map(|row| row.plain()).collect();
    assert!(
        plain
            .iter()
            .all(|row| !row.contains("Recent") && !row.contains("/resume"))
    );
}
