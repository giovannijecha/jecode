use super::*;

#[test]
fn skills_catalog_is_local_keeps_drafts_and_restores_its_saved_details() {
    let home = Directory::new();
    let directory = Directory::new();
    let skill = home.path().join("skills/review");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: review\ndescription: Inspect a change.\n---\n\n# Review\n",
    )
    .unwrap();
    let mut first = app(&home, &directory, "http://127.0.0.1:1/chat/completions");
    saved_exchange(&first);
    let messages_before = first.archive.messages.lock().unwrap().len();
    first.state.editor.replace("unfinished draft".into());
    first.state.queue.push("queued draft").unwrap();
    first.persist_input(true);
    first.dispatch("/skills".into()).unwrap();
    assert_eq!(first.state.editor.text, "unfinished draft");
    assert_eq!(first.state.queue.messages()[0].text, "queued draft");
    assert!(first.worker.is_none());
    assert_eq!(
        first.archive.messages.lock().unwrap().len(),
        messages_before
    );
    let info = first.state.information.as_ref().unwrap();
    assert_eq!(info.title, "Skills");
    assert_eq!(info.current().aside, "1 available skill");
    assert_eq!(
        info.current().details,
        [("review".into(), "Inspect a change.".into())]
    );
    let id = first.persistence.as_ref().unwrap().id();
    drop(first);

    let mut resumed = app(&home, &directory, "http://127.0.0.1:1/chat/completions");
    resumed.resume_session(&id);
    assert!(resumed.state.items.iter().any(|item| matches!(
        item,
        state::Item::Local { command, details, .. }
        if command == "/skills" && details == &[("review".into(), "Inspect a change.".into())]
    )));
    assert_eq!(resumed.state.editor.text, "unfinished draft");
    assert_eq!(resumed.state.queue.paused[0].text, "queued draft");
    assert!(resumed.worker.is_none());
}

#[test]
fn skills_catalog_shows_empty_and_malformed_states_without_provider_work() {
    let home = Directory::new();
    let directory = Directory::new();
    let mut current = app(&home, &directory, "http://127.0.0.1:1/chat/completions");
    current.dispatch("/skills".into()).unwrap();
    assert!(current.state.information.is_none());
    assert!(current.state.items.iter().any(|item| matches!(
        item,
        state::Item::Local { command, result: Some(result), .. }
        if command == "/skills" && result == "No skills installed."
    )));
    let malformed = home.path().join("skills/broken");
    std::fs::create_dir_all(&malformed).unwrap();
    std::fs::write(malformed.join("SKILL.md"), "invalid").unwrap();
    current.dispatch("/skills".into()).unwrap();
    let info = current.state.information.as_ref().unwrap();
    assert!(info.current().aside.contains("No valid skills found"));
    assert!(
        info.current()
            .details
            .iter()
            .any(|(name, _)| name == "Skipped")
    );
    assert!(current.worker.is_none());
}

#[test]
fn loaded_skill_card_replays_from_tool_history_after_resume() {
    let home = Directory::new();
    let directory = Directory::new();
    let skill = home.path().join("skills/review");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: review\ndescription: Inspect a change.\n---\n\n# Review\nCheck changes.\n",
    )
    .unwrap();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "Loading the skill.",
                vec![tool_call(
                    "skill-read",
                    "read",
                    Value::object([("path", Value::string("skill:review"))]),
                )],
            ),
        ),
        (200, completion("Done.", vec![])),
    ]);
    let mut first = app(&home, &directory, &fixture.endpoint);
    submit(&mut first, "Read the review skill");
    finish(&mut first);
    let rendered = first
        .state
        .rows()
        .into_iter()
        .flatten()
        .map(|line| line.plain())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rendered.contains(" SKILL  review"), "{rendered}");
    assert!(rendered.contains("Loaded skill"), "{rendered}");
    let id = first.persistence.as_ref().unwrap().id();
    drop(first);
    assert_eq!(fixture.finish().len(), 2);

    let mut resumed = app(&home, &directory, "http://127.0.0.1:1/chat/completions");
    resumed.resume_session(&id);
    let rows = resumed
        .state
        .rows()
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    assert!(
        rows.iter()
            .any(|row| row.plain().contains(" SKILL  review"))
    );
    assert!(rows.iter().any(|row| row.plain().contains("Loaded skill")));
    // A replayed successful skill keeps the read-family success badge.
    assert!(rows.iter().any(|row| {
        row.plain().starts_with(" SKILL  review")
            && row.spans[0].style == crate::tui::theme::BADGE_TEXT.bold()
            && row.spans[0].background == Some(crate::tui::theme::BADGE_READ)
    }));
}

#[test]
fn queued_skills_command_runs_locally_after_the_active_turn() {
    let home = Directory::new();
    let directory = Directory::new();
    let skill = home.path().join("skills/review");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: review\ndescription: Inspect a change.\n---\n\n# Review\n",
    )
    .unwrap();
    let fixture = HttpFixture::new(vec![(200, completion("Answer.", vec![]))]);
    let mut current = app(&home, &directory, &fixture.endpoint);
    submit(&mut current, "First question");
    assert!(current.worker.is_some());
    submit(&mut current, "/skills");
    finish(&mut current);
    assert!(current.state.items.iter().any(|item| matches!(
        item,
        state::Item::Local { command, details, .. }
        if command == "/skills" && details.iter().any(|(name, _)| name == "review")
    )));
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn init_keeps_its_command_label_after_resume_without_rewriting_the_model_request() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Instructions inspected.", vec![]))]);
    let mut first = app(&home, &directory, &fixture.endpoint);
    submit(&mut first, "/init");
    finish(&mut first);
    assert!(first.state.items.iter().any(|item| matches!(
        item,
        state::Item::Text { kind: Kind::User, text } if text == "/init"
    )));
    let id = first.persistence.as_ref().unwrap().id();
    drop(first);
    assert_eq!(fixture.finish().len(), 1);

    let mut resumed = app(&home, &directory, "http://127.0.0.1:1/chat/completions");
    resumed.resume_session(&id);
    assert!(resumed.state.items.iter().any(|item| matches!(
        item,
        state::Item::Text { kind: Kind::User, text } if text == "/init"
    )));
    assert!(
        resumed
            .archive
            .messages
            .lock()
            .unwrap()
            .iter()
            .any(|message| {
                message.get("role").and_then(Value::as_str) == Some("user")
                    && message.get("content").and_then(Value::as_str)
                        == Some(crate::session::commands::INIT_PROMPT)
            })
    );
    assert!(resumed.state.history.snapshot().is_empty());
}
