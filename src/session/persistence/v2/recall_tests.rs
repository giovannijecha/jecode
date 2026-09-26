use super::*;
use crate::session::{self, End, Event, Session};

fn measured_head(store: &Store, id: &str) -> (usize, usize, usize, usize) {
    let data = store.directory("sessions-v2").unwrap();
    let source = data
        .read(&format!("{id}.head"), head::HEAD_LIMIT)
        .unwrap()
        .unwrap();
    let value = json::parse(
        &source,
        json::Limits {
            bytes: head::HEAD_LIMIT,
            nodes: 100_000,
            depth: 32,
        },
    )
    .unwrap();
    let encoded = |key| {
        json::encode(value.get(key).unwrap(), head::HEAD_LIMIT)
            .unwrap()
            .len()
    };
    assert!(source.len() <= head::HEAD_LIMIT);
    (
        source.len(),
        encoded("title"),
        encoded("projection"),
        encoded("recent"),
    )
}

fn mixed_prompt(index: usize) -> String {
    // This fixture exercises the historical 8 KiB prompt/head shape. The
    // application limit is larger, but repeating 256 KiB 72 times would test
    // context exhaustion rather than recall metadata.
    const FIXTURE_PROMPT_BYTES: usize = 8192;
    let mut prompt = format!("{index:04}");
    let content = "\"\\\r\n🌍";
    while prompt.len() + content.len() <= FIXTURE_PROMPT_BYTES {
        prompt.push_str(content);
    }
    prompt.push_str(&"x".repeat(FIXTURE_PROMPT_BYTES - prompt.len()));
    assert_eq!(prompt.len(), FIXTURE_PROMPT_BYTES);
    prompt
}

#[test]
fn a_256_kib_prompt_survives_submission_checkpoint_and_resume() {
    use crate::providers::openai_account::{Progress, Request, Response, Status, client};
    use crate::tls::Budget;
    use std::ops::ControlFlow;
    struct Answer;
    impl session::worker::Backend for Answer {
        fn login(
            &mut self,
            _: &Budget<'_>,
            _: &mut dyn FnMut(&str) -> ControlFlow<()>,
        ) -> Result<(), client::Error> {
            Ok(())
        }
        fn generate(
            &mut self,
            _: &Request,
            _: &Budget<'_>,
            _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
        ) -> Result<Response, client::Error> {
            Ok(session::tests::response("saved answer", Status::Completed))
        }
    }
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let history = create(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    let prompt = format!("{}done", "🌍".repeat((session::MAX_PROMPT_BYTES - 4) / 4));
    assert_eq!(prompt.len(), session::MAX_PROMPT_BYTES);
    let mut run = Session::with_history(Model::Luna, Answer, None, history).unwrap();
    assert!(matches!(
        session::tests::next(&mut run),
        Event::Restored { .. }
    ));
    assert!(matches!(session::tests::next(&mut run), Event::Ready));
    assert!(run.submit(&prompt));
    loop {
        if let Event::Finished(end, _) = session::tests::next(&mut run) {
            assert_eq!(end, End::Complete);
            break;
        }
    }
    drop(run);
    let saved = super::super::load(&store, &id, true).unwrap();
    assert_eq!(saved.turns, 1);
    assert_eq!(saved.history.turns[0].prompt, prompt);
    assert_eq!(saved.history.transcript()[0].text, prompt);
    assert!(saved.title.len() <= 8192);
    assert_eq!(saved.title, &prompt[..8192]);
}

#[test]
fn astra_valid_prompt_recall_does_not_stop_canonical_checkpoints() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    let mut prompts = Vec::new();
    for index in 0..64 {
        let prompt = format!("{index:04}{}", "\"".repeat(8188));
        assert_eq!(prompt.len(), 8192);
        history.begin(prompt.clone()).unwrap();
        prompts.push(prompt);
        let turn = history.turns.last_mut().unwrap();
        turn.end = Some(End::Complete);
        turn.outcome = "Complete".into();
        let result = history.checkpoint();
        assert!(
            result.is_ok(),
            "valid turn {} failed checkpoint: {:?}",
            index + 1,
            result
        );
        history.projection.through = history.turns.len();
        history.projection.summary = "Earlier prompts completed.".into();
        history.checkpoint().unwrap();
        history.release_projected();
    }
    // Both fields are valid bounded projection data. Their escaped bytes and
    // the first prompt's title must be accounted for before retaining recall.
    let summary = "\"".repeat(32768);
    let partial = "\\".repeat(32768);
    history.projection.summary = summary.clone();
    history.projection.failed_partial = partial.clone();
    history.checkpoint().unwrap();
    let before = measured_head(&store, &id);
    eprintln!(
        "recall head after 64: total={} title={} projection={} recent={} limit={}",
        before.0,
        before.1,
        before.2,
        before.3,
        head::HEAD_LIMIT
    );
    drop(history);
    let saved = super::super::load(&store, &id, true).unwrap();
    assert_eq!(saved.turns, 64);
    assert_eq!(saved.title, prompts[0]);
    assert_eq!(saved.history.projection.summary, summary);
    assert_eq!(saved.history.projection.failed_partial, partial);
    let recalled = saved
        .history
        .record
        .as_ref()
        .unwrap()
        .recent_prompts()
        .unwrap();
    assert!(!recalled.is_empty() && recalled.len() < 64);
    assert_eq!(recalled, prompts[64 - recalled.len()..]);

    use crate::providers::openai_account::{Progress, Request, Response, Status, client};
    use crate::tls::Budget;
    use std::ops::ControlFlow;
    struct Answer;
    impl session::worker::Backend for Answer {
        fn login(
            &mut self,
            _: &Budget<'_>,
            _: &mut dyn FnMut(&str) -> ControlFlow<()>,
        ) -> Result<(), client::Error> {
            Ok(())
        }
        fn generate(
            &mut self,
            _: &Request,
            _: &Budget<'_>,
            _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
        ) -> Result<Response, client::Error> {
            Ok(session::tests::response("done", Status::Completed))
        }
    }
    let mut run = Session::with_history(Model::Luna, Answer, None, saved.history).unwrap();
    assert_eq!(run.take_initial_prompts(), recalled);
    assert!(matches!(
        session::tests::next(&mut run),
        Event::Restored { .. }
    ));
    assert!(matches!(session::tests::next(&mut run), Event::Ready));
    for index in 64..72 {
        let prompt = mixed_prompt(index);
        assert!(run.submit(&prompt));
        prompts.push(prompt);
        loop {
            if let Event::Finished(end, _) = session::tests::next(&mut run) {
                assert_eq!(end, End::Complete, "turn {index}");
                break;
            }
        }
    }
    drop(run);
    let after = measured_head(&store, &id);
    eprintln!(
        "recall head after 72: total={} title={} projection={} recent={} limit={}",
        after.0,
        after.1,
        after.2,
        after.3,
        head::HEAD_LIMIT
    );
    let again = super::super::load(&store, &id, true).unwrap();
    assert_eq!(again.turns, 72);
    let recalled = again
        .history
        .record
        .as_ref()
        .unwrap()
        .recent_prompts()
        .unwrap();
    assert!(!recalled.is_empty() && recalled.len() < 64);
    assert_eq!(recalled, prompts[72 - recalled.len()..]);
    for start in (0..72).step_by(16) {
        let turns = page(
            again.history.record.as_ref().unwrap(),
            start,
            16.min(72 - start),
        )
        .unwrap();
        for (index, turn) in turns.iter().enumerate() {
            assert_eq!(turn.prompt, prompts[start + index]);
            assert_eq!(turn.end, Some(End::Complete));
        }
    }
}

#[test]
fn importing_legacy_prompts_keeps_source_and_exact_canonical_text_after_recall_eviction() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let directory = crate::session::scope::Directory::open(&fixture.0).unwrap();
    let mut legacy =
        super::super::create_legacy_in(&store, Model::Luna, Some(directory.path()), None).unwrap();
    let old_id = legacy.record.as_ref().unwrap().id().to_owned();
    let prompts: Vec<_> = (0..64)
        .map(|index| format!("{index:04}{}", "\"".repeat(8188)))
        .collect();
    for prompt in &prompts {
        legacy.begin(prompt.clone()).unwrap();
        let turn = legacy.turns.last_mut().unwrap();
        turn.end = Some(End::Complete);
        turn.outcome = "Complete".into();
    }
    legacy.checkpoint().unwrap();
    drop(legacy);
    let source = store
        .directory("sessions")
        .unwrap()
        .root()
        .join(format!("{old_id}.json"));
    let before = std::fs::read(&source).unwrap();
    let new_id = super::super::import_in_store(&store, &old_id, &directory).unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), before);
    let imported = super::super::load(&store, &new_id, true).unwrap();
    let recalled = imported
        .history
        .record
        .as_ref()
        .unwrap()
        .recent_prompts()
        .unwrap();
    assert!(!recalled.is_empty() && recalled.len() < 64);
    assert_eq!(recalled, prompts[64 - recalled.len()..]);
    for start in (0..64).step_by(16) {
        let turns = page(imported.history.record.as_ref().unwrap(), start, 16).unwrap();
        for (index, turn) in turns.iter().enumerate() {
            assert_eq!(turn.prompt, prompts[start + index]);
        }
    }
}
