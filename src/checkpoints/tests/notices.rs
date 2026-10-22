use super::*;
use crate::checkpoints::record::{Event, Record};
use crate::checkpoints::session::Session;

#[test]
fn delayed_notices_are_bounded_and_report_released_instruction_events() {
    let fixture = Fixture::new();
    fixture.write("JECODE.md", b"project instructions\n");
    let session = Session::offline(fixture.home.path(), fixture.root.path());
    session.begin("idle-conversation", None);
    let workspace = session.workspace().unwrap();
    workspace
        .store()
        .transaction(true, |transaction| {
            for number in 0..300 {
                let seq = transaction.seq();
                transaction.record(Record::Event(Event {
                    seq,
                    origin: "cli".into(),
                    time: crate::checkpoints::clock::now(),
                    text: "restored ".repeat(500),
                    paths: if number == 0 {
                        vec!["JECODE.md".into()]
                    } else {
                        vec![format!("file-{number}")]
                    },
                }));
            }
            Ok(())
        })
        .unwrap();
    let notice = session.notices().unwrap();
    assert!(
        notice.instructions,
        "the released event may have changed instructions"
    );
    assert!(
        notice.text.len() < 12_000,
        "notices must fit a bounded request context"
    );
    assert!(notice.text.contains("Older workspace notices"));
    assert!(notice.text.contains("earlier events omitted"));
    assert!(session.notices().is_none(), "the notice is delivered once");
}
