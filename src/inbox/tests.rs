use super::*;
use crate::attachments::tests::attachment;

fn recorded(prompts: &[&str]) -> Inbox {
    let inbox = Inbox::default();
    inbox.update(|entries| {
        for prompt in prompts {
            entries.push((*prompt).into()).unwrap();
        }
        entries.record();
    });
    inbox
}

fn accept_all(inbox: &Inbox) -> Vec<Prompt> {
    inbox.accept(|_, _| Ok(())).unwrap()
}

#[test]
fn ordinary_messages_join_in_order_and_commands_wait_without_blocking_them() {
    let image = Prompt::new(
        "/look at this".into(),
        vec![attachment("att-1-1-1", "a.png", "image/png")],
    );
    let inbox = recorded(&["first", "/model", "second"]);
    inbox.update(|entries| {
        entries.push(image.clone()).unwrap();
        entries.record();
    });
    let mut remaining = Vec::new();
    let accepted = inbox
        .accept(|prompts, pending| {
            assert_eq!(prompts.len(), 3);
            remaining = pending;
            Ok(())
        })
        .unwrap();
    assert_eq!(accepted[..2], ["first", "second"]);
    assert_eq!(accepted[2], image);
    assert_eq!(remaining, ["/model"]);
    inbox.with(|entries| {
        assert_eq!(entries.unacknowledged(), accepted);
        assert!(!entries.ready());
        assert_eq!(entries.pop_front().unwrap(), "/model");
    });
}

#[test]
fn messages_wait_until_the_interface_records_them() {
    let inbox = Inbox::default();
    inbox.update(|entries| entries.push("draft".into()).unwrap());
    assert!(accept_all(&inbox).is_empty());
    inbox.with(|entries| assert_eq!(entries.record(), ["draft"]));
    assert_eq!(accept_all(&inbox), ["draft"]);
    // A later capture can no longer return an accepted message.
    inbox.with(|entries| assert!(entries.record().is_empty()));
}

#[test]
fn a_failed_acceptance_keeps_every_message_pending() {
    let inbox = recorded(&["first", "second"]);
    let error = inbox.accept(|_, _| Err("disk full".into())).unwrap_err();
    assert_eq!(error, "disk full");
    inbox.with(|entries| {
        assert_eq!(entries.len(), 2);
        assert!(entries.unacknowledged().is_empty());
    });
    assert_eq!(accept_all(&inbox), ["first", "second"]);
}

#[test]
fn an_edit_holds_its_message_and_later_ones_but_not_earlier_ones() {
    let inbox = recorded(&["first", "second", "third"]);
    let held = inbox.with(|entries| entries.prompts().nth(1).unwrap().0);
    inbox.update(|entries| {
        assert_eq!(entries.hold(held).unwrap(), "second");
        entries.revise("second revised".into());
        assert!(entries.remove(held).is_none());
        assert!(entries.pop_front().is_none());
    });
    let mut remaining = Vec::new();
    let accepted = inbox
        .accept(|_, pending| {
            remaining = pending;
            Ok(())
        })
        .unwrap();
    assert_eq!(accepted, ["first"]);
    // Saved input carries the unsaved revision of the held message.
    assert_eq!(remaining, ["second revised", "third"]);
    assert!(matches!(
        inbox.close(|_, _| Ok(())).unwrap(),
        Boundary::Held
    ));
    inbox.update(|entries| entries.release(Some("second saved".into())));
    let Boundary::Accepted(accepted) = inbox.close(|_, _| Ok(())).unwrap() else {
        panic!("released messages should join the turn");
    };
    assert_eq!(accepted, ["second saved", "third"]);
    assert!(matches!(
        inbox.close(|_, _| Ok(())).unwrap(),
        Boundary::Closed
    ));
}

#[test]
fn an_edited_command_does_not_hold_the_turn_open() {
    let inbox = recorded(&["/status"]);
    let held = inbox.with(|entries| entries.prompts().next().unwrap().0);
    inbox.update(|entries| entries.hold(held).unwrap());
    assert!(matches!(
        inbox.close(|_, _| Ok(())).unwrap(),
        Boundary::Closed
    ));
}

#[test]
fn waiting_for_an_edit_ends_on_release_or_cancellation() {
    let inbox = recorded(&["edited"]);
    let held = inbox.with(|entries| entries.prompts().next().unwrap().0);
    inbox.update(|entries| entries.hold(held).unwrap());
    let cancellation = Cancellation::default();
    let waiter = {
        let (inbox, cancellation) = (inbox.clone(), cancellation.clone());
        std::thread::spawn(move || inbox.wait(&cancellation))
    };
    inbox.update(|entries| entries.release(None));
    assert_eq!(waiter.join().unwrap(), Ok(()));

    inbox.update(|entries| entries.hold(held).unwrap());
    let waiter = {
        let (inbox, cancellation) = (inbox.clone(), cancellation.clone());
        std::thread::spawn(move || inbox.wait(&cancellation))
    };
    cancellation.cancel();
    inbox.wake();
    assert_eq!(waiter.join().unwrap().unwrap_err(), "Operation cancelled");
}

#[test]
fn the_queue_limit_and_draining_preserve_the_held_position() {
    let inbox = Inbox::default();
    inbox.update(|entries| {
        for index in 0..LIMIT {
            entries.push(index.to_string().into()).unwrap();
        }
        assert_eq!(
            entries.push("extra".into()).unwrap_err(),
            "Queue is full (8 messages). Your draft was kept."
        );
        let held = entries.prompts().nth(3).unwrap().0;
        entries.hold(held).unwrap();
        let (prompts, position) = entries.drain();
        assert_eq!(prompts.len(), LIMIT);
        assert_eq!(position, Some(3));
        assert!(entries.pop_front().is_none());
    });
}

#[test]
fn acknowledgement_leaves_only_undisplayed_messages() {
    let inbox = recorded(&["one", "two", "three"]);
    accept_all(&inbox);
    inbox.with(|entries| {
        entries.acknowledge(2);
        assert_eq!(entries.unacknowledged(), ["three"]);
        entries.acknowledge(5);
        assert!(entries.unacknowledged().is_empty());
    });
}
