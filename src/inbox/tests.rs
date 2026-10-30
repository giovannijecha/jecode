use super::*;
use crate::attachments::tests::attachment;

fn recorded(prompts: &[&str]) -> Inbox {
    let inbox = Inbox::default();
    inbox.with(|entries| {
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
    inbox.with(|entries| {
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
        assert_eq!(entries.pop_front().unwrap(), "/model");
    });
}

#[test]
fn messages_wait_until_the_interface_records_them() {
    let inbox = Inbox::default();
    inbox.with(|entries| entries.push("draft".into()).unwrap());
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
fn the_queue_limit_keeps_extra_prompts_out_and_draining_empties_it() {
    let inbox = Inbox::default();
    inbox.with(|entries| {
        for index in 0..LIMIT {
            entries.push(index.to_string().into()).unwrap();
        }
        assert_eq!(
            entries.push("extra".into()).unwrap_err(),
            "Queue is full (8 messages). Your draft was kept."
        );
        assert_eq!(entries.drain().len(), LIMIT);
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
