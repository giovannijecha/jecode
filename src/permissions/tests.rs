use super::*;
use std::thread;

fn access(kind: Kind, outside: bool) -> Access {
    Access {
        kind,
        target: "/elsewhere/session.rs".into(),
        place: None,
        outside,
        base: None,
        identity: None,
    }
}

fn allowed(permissions: &Permissions, access: &Access) -> &'static str {
    match permissions.check(access) {
        Verdict::Allow => "allow",
        Verdict::Unavailable(_) => "unavailable",
        Verdict::Ask => "ask",
    }
}

#[test]
fn levels_decide_inside_and_outside_access() {
    let permissions = Permissions::default();
    let cases = [
        (Kind::Reads, false, ["allow", "allow", "allow"]),
        (Kind::Reads, true, ["unavailable", "ask", "allow"]),
        (Kind::Edits, false, ["unavailable", "allow", "allow"]),
        (Kind::Edits, true, ["unavailable", "ask", "allow"]),
        (Kind::Runs, false, ["unavailable", "allow", "allow"]),
        (Kind::Runs, true, ["unavailable", "ask", "allow"]),
    ];
    for (kind, outside, expected) in cases {
        for (level, verdict) in Level::ALL.into_iter().zip(expected) {
            permissions.set_level(level).unwrap();
            assert_eq!(
                allowed(&permissions, &access(kind, outside)),
                verdict,
                "{kind:?} outside={outside} at {level:?}"
            );
        }
    }
}

#[test]
fn remembered_grants_extend_workspace_only_and_match_exactly() {
    let permissions = Permissions::default();
    permissions.set_surface(Surface::Panel);
    let read = access(Kind::Reads, true);
    let request = permissions.begin("call-1", "read", &read).unwrap();
    permissions.decide(request.id, Decision::Remember).unwrap();
    assert!(matches!(
        permissions.wait(request.id, &Cancellation::default()),
        Asked::Decided(Decision::Remember)
    ));
    assert_eq!(allowed(&permissions, &read), "allow");
    // Another access to the same file, or the same access elsewhere, asks.
    assert_eq!(allowed(&permissions, &access(Kind::Edits, true)), "ask");
    let other = Access {
        target: "/elsewhere/other.rs".into(),
        ..read.clone()
    };
    assert_eq!(allowed(&permissions, &other), "ask");
    // Paused at Read, applied again back at Workspace.
    permissions.set_level(Level::Read).unwrap();
    assert_eq!(allowed(&permissions, &read), "unavailable");
    permissions.set_level(Level::Workspace).unwrap();
    assert_eq!(allowed(&permissions, &read), "allow");
    permissions.revoke(&read.grant()).unwrap();
    assert_eq!(allowed(&permissions, &read), "ask");
    assert!(permissions.revoke(&read.grant()).is_err());
}

#[test]
fn requests_queue_in_arrival_order_and_only_the_asker_waits() {
    let permissions = Permissions::default();
    assert!(
        permissions
            .begin("call-0", "read", &access(Kind::Reads, true))
            .is_none(),
        "no surface"
    );
    permissions.set_surface(Surface::Inline);
    assert!(
        permissions
            .for_agent("a1")
            .begin("call-0", "read", &access(Kind::Reads, true))
            .is_none(),
        "the line-based CLI shows the main agent's requests only"
    );
    permissions.set_surface(Surface::Panel);
    let collaborator = permissions.for_agent("a1");
    let waiter = thread::spawn(move || {
        let request = collaborator
            .begin("child-call", "bash", &access(Kind::Runs, true))
            .unwrap();
        collaborator.wait(request.id, &Cancellation::default())
    });
    while permissions.pending().is_none() {
        thread::yield_now();
    }
    let main = permissions
        .begin("main-call", "read", &access(Kind::Reads, true))
        .unwrap();
    let (first, count) = permissions.pending().unwrap();
    assert_eq!((first.agent.as_deref(), count), (Some("a1"), 2));
    permissions.decide(first.id, Decision::Deny).unwrap();
    assert!(matches!(
        waiter.join().unwrap(),
        Asked::Decided(Decision::Deny)
    ));
    assert_eq!(permissions.pending().unwrap().0, main);

    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert!(matches!(
        permissions.wait(main.id, &cancellation),
        Asked::Cancelled
    ));
    assert!(permissions.pending().is_none());
}

#[test]
fn replay_restores_the_last_level_and_live_grants() {
    let read = access(Kind::Reads, true).grant();
    let run = Grant {
        kind: Kind::Runs,
        target: "cargo test".into(),
        place: Some("/elsewhere".into()),
        identity: None,
    };
    let events = [
        journal::level(Level::Full),
        journal::grant("permission_grant", &read),
        journal::grant("permission_grant", &run),
        journal::grant("permission_revoke", &read),
        journal::level(Level::Workspace),
    ];
    assert_eq!(replay(&events), (Some(Level::Workspace), vec![run]));
    assert_eq!(replay(&[]), (None, vec![]));
}

#[test]
fn a_request_without_a_decision_is_pending_at_quit() {
    let request = Request {
        id: 1,
        call_id: "call-1".into(),
        agent: None,
        tool: "read".into(),
        access: access(Kind::Reads, true),
    };
    let asked = journal::request(&request);
    assert!(pending_request(std::slice::from_ref(&asked), "call-1"));
    assert!(!pending_request(std::slice::from_ref(&asked), "call-2"));
    let decided = [asked, journal::decision(&request, Decision::Once)];
    assert!(!pending_request(&decided, "call-1"));
}

#[test]
fn a_remembered_decision_replays_as_one_event_and_revoke_removes_it() {
    let request = Request {
        id: 1,
        call_id: "call-1".into(),
        agent: None,
        tool: "exec".into(),
        access: Access {
            kind: Kind::Runs,
            target: "cargo test".into(),
            place: Some("/elsewhere".into()),
            outside: true,
            base: None,
            identity: Some("digest".into()),
        },
    };
    let asked = journal::request(&request);
    assert_eq!(replay(std::slice::from_ref(&asked)).1, vec![]);
    assert!(pending_request(std::slice::from_ref(&asked), "call-1"));
    let decided = journal::decision(&request, Decision::Remember);
    let events = [asked, decided];
    assert_eq!(replay(&events).1, vec![request.access.grant()]);
    assert!(!pending_request(&events, "call-1"));
    let revoked = [
        events[0].clone(),
        events[1].clone(),
        journal::grant("permission_revoke", &request.access.grant()),
    ];
    assert!(replay(&revoked).1.is_empty());
}

#[test]
fn command_identity_survives_redacted_display_text_without_widening_the_grant() {
    let permissions = Permissions::default();
    let mut requested = access(Kind::Runs, true);
    requested.target = "program --token private-value".into();
    requested.place = Some("/elsewhere".into());
    requested.identity = Some("exact-command-digest".into());
    let mut remembered = requested.grant();
    remembered.target = "program --token [REDACTED]".into();
    permissions.state().grants.push(remembered);
    assert_eq!(allowed(&permissions, &requested), "allow");
    requested.identity = Some("different-command-digest".into());
    assert_eq!(allowed(&permissions, &requested), "ask");
    requested.identity = None;
    assert_eq!(allowed(&permissions, &requested), "ask");
}
