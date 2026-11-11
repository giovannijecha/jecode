use super::*;
use crate::{agent::Agent, backend::openrouter::OpenRouter, test_support::Directory, tools::Tools};

#[test]
fn creation_records_fifo_without_starting_a_thread_for_each_assignment() {
    let project = Directory::new();
    let home = Directory::new();
    let mut parent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    let observer = parent.collaboration();
    let total = slots::RUNNING + 5;
    let ids = (0..total)
        .map(|index| {
            observer
                .create(
                    project.path(),
                    Spec {
                        title: format!("Inspect {index}"),
                        instructions: "Report".into(),
                        context: String::new(),
                        mode: workspace::Mode::Shared,
                    },
                    |_, root| {
                        Ok(Agent::new(
                            OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
                            Tools::new(root)?,
                        ))
                    },
                )
                .unwrap()
                .get("id")
                .and_then(Value::as_str)
                .unwrap()
                .to_owned()
        })
        .collect::<Vec<_>>();
    {
        let state = observer.0.state.lock().unwrap();
        assert_eq!(state.running, 0);
        assert!(state.workers.iter().all(|worker| !worker.busy));
        assert_eq!(state.queue.iter().cloned().collect::<Vec<_>>(), ids);
    }
    observer.abort_created(&ids);
    assert!(observer.snapshot().is_empty());
}
#[test]
fn cancelled_queued_assignment_stops_without_receiving() {
    let project = Directory::new();
    let home = Directory::new();
    let mut parent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    let observer = parent.collaboration();
    observer
        .create(
            project.path(),
            Spec {
                title: "Inspect".into(),
                instructions: "Report".into(),
                context: String::new(),
                mode: workspace::Mode::Shared,
            },
            |_, root| {
                Ok(Agent::new(
                    OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
                    Tools::new(root)?,
                ))
            },
        )
        .unwrap();
    observer
        .receipt("created", None, 0, "Agents", vec!["a1".into()])
        .unwrap();
    observer.0.state.lock().unwrap().workers[0]
        .cancellation
        .cancel();
    observer.release_created(&["a1".into()]);
    let record = &observer.snapshot()[0];
    assert_eq!(record.get("state").and_then(Value::as_str), Some("stopped"));
    assert_eq!(record.get("received_version"), Some(&Value::number(0)));
    assert_eq!(observer.slot_stats(), (0, vec![], 0));
    assert!(
        !observer
            .events_since(0)
            .1
            .iter()
            .any(|event| { event.get("kind").and_then(Value::as_str) == Some("received") })
    );
}
