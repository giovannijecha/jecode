use super::*;
use crate::events::Event;

#[test]
fn extreme_retry_after_keeps_progress_without_an_overflowing_countdown() {
    let mut state = State::default();
    state.event(Event::Recovering {
        attempt: 1,
        delay: Duration::from_secs(u64::MAX),
        error: "provider requested a long retry delay".into(),
        reason: Some(crate::backend::FailureReason::RateLimited),
    });
    let feedback = state.current_progress().unwrap();
    assert!(feedback.in_progress());
    assert!(feedback.text.starts_with("Rate limited"));
    assert!(feedback.until.is_none());
    state.event(Event::RecoveryFinished);
    assert!(state.current_progress().is_none());
}

#[test]
fn a_new_preparation_discards_stale_backend_failure_metadata() {
    let mut state = State::default();
    state.event(Event::BackendFailure {
        reason: crate::backend::FailureReason::RejectedKey,
        diagnostic: "Previous failure".into(),
    });
    assert!(state.backend_failure.is_some());
    state.event(Event::Preparing);
    assert!(state.backend_failure.is_none());
}

#[test]
fn retry_progress_temporarily_overlays_maintenance_and_restores_its_clock() {
    let mut state = State::default();
    state.event(Event::Maintenance {
        text: "Compacting context".into(),
    });
    for attempt in 1..=2 {
        state.event(Event::Recovering {
            attempt,
            delay: Duration::from_secs(1),
            error: "provider asked us to retry".into(),
            reason: Some(crate::backend::FailureReason::RateLimited),
        });
        assert!(
            state
                .current_progress()
                .unwrap()
                .text
                .starts_with("Rate limited")
        );
    }
    state.event(Event::RecoveryFinished);
    let restored = state
        .current_progress()
        .expect("maintenance outlives a retry");
    assert_eq!(restored.text, "Compacting context");
    state.event(Event::ContextCompacted {
        text: "Context compacted".into(),
    });
    assert!(state.current_progress().is_none());
    state.event(Event::Recovering {
        attempt: 1,
        delay: Duration::from_secs(1),
        error: "a diagnostic mentions HTTP 429 without a rate limit".into(),
        reason: None,
    });
    state.event(Event::Maintenance {
        text: "Compacting context".into(),
    });
    assert!(
        state
            .current_progress()
            .unwrap()
            .text
            .starts_with("Reconnecting")
    );
    state.clear_progress();
    state.event(Event::RecoveryFinished);
    assert!(
        state.current_progress().is_none(),
        "finishing a retry must not resurrect cleared work"
    );
}

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
    assert!(state.notice.is_some() && state.copy_notice.is_some());
    state.sent();
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
        copy_notice: Some(Feedback::at(Kind::Notice, "Copied", now)),
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
fn errors_wait_for_the_next_send_and_progress_waits_for_its_operation() {
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
    state.feedback_input(true);
    assert!(state.notice.is_some());
    state.sent();
    assert!(state.notice.is_none());
    assert!(state.copy_notice.is_some());
}

#[test]
fn warnings_wait_for_a_key_and_yield_only_to_equal_or_greater_severity() {
    let now = Instant::now();
    let mut state = State::default();
    state.notify(Feedback::at(Kind::Warning, "Interrupted", now));
    assert!(!state.expire_feedback(now + Duration::from_secs(600)));
    state.notify(Feedback::result(Kind::Notice, "Complete"));
    assert_eq!(state.notice.as_ref().unwrap().text, "Interrupted");
    state.notify(Feedback::result(Kind::Warning, "Rate limited"));
    assert_eq!(state.notice.as_ref().unwrap().text, "Rate limited");
    state.notify(Feedback::result(Kind::Error, "HTTP 401"));
    state.notify(Feedback::result(Kind::Warning, "Interrupted"));
    assert_eq!(state.notice.as_ref().unwrap().text, "HTTP 401");
    state.feedback_input(false);
    assert_eq!(state.notice.as_ref().unwrap().text, "HTTP 401");
    state.notify(Feedback::result(Kind::Warning, "Rate limited"));
    state.sent();
    assert!(state.notice.is_none());
    state.notify(Feedback::result(Kind::Warning, "Rate limited"));
    state.feedback_input(false);
    assert!(state.notice.is_none());
}

#[test]
fn progress_has_its_own_slot_beside_a_result_and_ends_with_its_operation() {
    let mut state = State::default();
    state.notify(Feedback::result(Kind::Notice, "Attached 1 item"));
    state.notify(Feedback::progress(Kind::Notice, "Compacting context"));
    assert_eq!(state.notice.as_ref().unwrap().text, "Attached 1 item");
    assert_eq!(state.current_progress().unwrap().text, "Compacting context");
    state.feedback_input(true);
    assert!(state.notice.is_none());
    assert!(state.current_progress().is_some());
    state.clear_progress();
    assert!(state.current_progress().is_none());
    state.notify_copy(Feedback::progress(Kind::Notice, "Copying…"));
    assert_eq!(state.current_progress().unwrap().text, "Copying…");
}

#[test]
fn each_result_has_its_own_deadline_and_wakes_the_idle_event_loop() {
    let now = Instant::now();
    let mut state = State {
        notice: Some(Feedback::at(Kind::Notice, "Complete", now)),
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
