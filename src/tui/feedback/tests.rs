use super::*;

#[test]
fn automatic_success_and_progress_cannot_replace_an_unread_error() {
    let mut state = State::default();
    state.notify(Feedback::result(Kind::Error, "Autosave failed"));
    state.notify_copy(Feedback::result(Kind::Error, "Copy failed"));
    for kind in [Kind::Notice, Kind::Warning] {
        state.notify(Feedback::result(kind, "Complete"));
        state.notify(Feedback::progress(kind, "Reconnecting"));
        state.notify_copy(Feedback::result(kind, "Copied"));
        state.notify_copy(Feedback::progress(kind, "Copying"));
    }
    state.clear_progress();
    state.clear();
    assert_eq!(state.notice.as_ref().unwrap().text, "Autosave failed");
    assert_eq!(state.copy_notice.as_ref().unwrap().text, "Copy failed");
    state.feedback_input(false);
    state.notify(Feedback::result(Kind::Notice, "Complete"));
    state.notify_copy(Feedback::result(Kind::Notice, "Copied"));
    assert_eq!(state.notice.as_ref().unwrap().text, "Complete");
    assert_eq!(state.copy_notice.as_ref().unwrap().text, "Copied");
}

#[test]
fn brief_results_expire_at_five_seconds_or_when_editing_starts() {
    let now = Instant::now();
    let mut state = State {
        notice: Some(Feedback::at(Kind::Notice, "Previous session", now)),
        copy_notice: Some(Feedback::at(Kind::Warning, "Copy sent to terminal", now)),
        ..State::default()
    };
    state.feedback_input(false);
    assert!(state.notice.is_some());
    assert!(state.copy_notice.is_some());
    assert_eq!(state.feedback_wait(now, Duration::from_secs(30)), BRIEF);
    assert!(!state.expire_feedback(now + BRIEF - Duration::from_millis(1)));
    assert!(state.expire_feedback(now + BRIEF));
    assert!(state.notice.is_none());
    assert!(state.copy_notice.is_none());
    assert!(!state.expire_feedback(now + BRIEF));
    state.notice = Some(Feedback::at(Kind::Notice, "Defaults saved", now));
    state.copy_notice = Some(Feedback::at(Kind::Notice, "Copied", now));
    state.feedback_input(true);
    assert!(state.notice.is_none());
    assert!(state.copy_notice.is_none());
}

#[test]
fn errors_wait_for_user_action_and_progress_waits_for_its_operation() {
    let now = Instant::now();
    let mut state = State {
        notice: Some(Feedback::at(Kind::Error, "Autosave failed", now)),
        copy_notice: Some(Feedback::progress(Kind::Notice, "Copying…")),
        ..State::default()
    };
    assert!(!state.expire_feedback(now + Duration::from_secs(600)));
    assert_eq!(
        state.feedback_wait(now, Duration::from_secs(30)),
        Duration::from_secs(30)
    );
    state.feedback_input(false);
    assert!(state.notice.is_none());
    assert!(state.copy_notice.is_some());
    state.feedback_input(true);
    assert!(state.copy_notice.is_some());
}

#[test]
fn each_result_has_its_own_deadline_and_wakes_the_idle_event_loop() {
    let now = Instant::now();
    let mut state = State {
        notice: Some(Feedback::at(Kind::Warning, "Session warning", now)),
        copy_notice: Some(Feedback::at(
            Kind::Notice,
            "Copied",
            now + Duration::from_secs(2),
        )),
        ..State::default()
    };
    assert_eq!(
        state.feedback_wait(now, Duration::from_millis(20)),
        Duration::from_millis(20)
    );
    assert!(state.expire_feedback(now + BRIEF));
    assert!(state.notice.is_none());
    assert!(state.copy_notice.is_some());
    assert_eq!(
        state.feedback_wait(now + BRIEF, Duration::from_secs(30)),
        Duration::from_secs(2)
    );
}
