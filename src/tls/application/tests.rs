use super::*;
fn pair() -> (Application, Sender, Receiver) {
    (
        Application::new(Secret([1; 32]), Secret([2; 32])),
        Sender::new(&Secret([2; 32])),
        Receiver::new(&Secret([1; 32])),
    )
}
#[test]
fn key_update_changes_epochs_and_replies_under_the_old_sending_key() {
    let (mut app, mut sender, mut receiver) = pair();
    let request = sender
        .seal(ContentType::Handshake, &[24, 0, 0, 1, 1])
        .unwrap();
    let Incoming::Reply(reply) = app.receive(&request).unwrap() else {
        panic!("update reply");
    };
    assert_eq!(receiver.open(&reply).unwrap().bytes, [24, 0, 0, 1, 0]);
    let mut next_sender = Sender::new(&label::<32>(&[2; 32], b"traffic upd", &[]));
    let mut next_receiver = Receiver::new(&label::<32>(&[1; 32], b"traffic upd", &[]));
    let encoded = next_sender
        .seal(ContentType::Application, b"new server key")
        .unwrap();
    let Incoming::Data(data) = app.receive(&encoded).unwrap() else {
        panic!("application");
    };
    assert_eq!(data.bytes, b"new server key");
    assert_eq!(
        next_receiver
            .open(&app.send(b"new client key").unwrap())
            .unwrap()
            .bytes,
        b"new client key"
    );
    assert!(
        app.receive(&sender.seal(ContentType::Application, b"old key").unwrap())
            .is_err()
    );
}
#[test]
fn tickets_can_fragment_but_application_cannot_interrupt_a_handshake() {
    let (mut app, mut sender, _) = pair();
    let ticket = [4, 0, 0, 14, 0, 0, 0, 30, 0, 0, 0, 0, 0, 0, 1, 42, 0, 0];
    for byte in ticket {
        assert!(matches!(
            app.receive(&sender.seal(ContentType::Handshake, &[byte]).unwrap())
                .unwrap(),
            Incoming::Continue
        ));
    }
    assert!(matches!(
        app.receive(&sender.seal(ContentType::Application, b"body").unwrap())
            .unwrap(),
        Incoming::Data(_)
    ));
    app.receive(&sender.seal(ContentType::Handshake, &[24, 0]).unwrap())
        .unwrap();
    assert!(
        app.receive(
            &sender
                .seal(ContentType::Application, b"interleaved")
                .unwrap()
        )
        .is_err()
    );
}
#[test]
fn invalid_post_handshake_messages_and_alerts_do_not_look_like_success() {
    for bytes in [
        &[24, 0, 0, 1, 2][..],
        &[24, 0, 0, 0],
        &[24, 0, 0, 1, 0, 24],
        &[13, 0, 0, 0],
        &[4, 0, 0, 1, 0],
    ] {
        let (mut app, mut sender, _) = pair();
        assert!(
            app.receive(&sender.seal(ContentType::Handshake, bytes).unwrap())
                .is_err()
        );
    }
    let (mut app, mut sender, _) = pair();
    assert!(matches!(
        app.receive(&sender.seal(ContentType::Alert, &[1, 0]).unwrap())
            .unwrap(),
        Incoming::Close
    ));
    let (mut app, mut sender, _) = pair();
    assert!(matches!(
        app.receive(&sender.seal(ContentType::Alert, &[2, 40]).unwrap()),
        Err(Error::PeerAlert)
    ));
}
