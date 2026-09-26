//! Regressions at the real input, session-event and canonical-data boundaries.
use super::*;
use crate::{
    providers::openai_account::{Progress, Request, Response, Status, catalog::Catalog, client},
    session::{self, Event, Session, TestBackend},
    tls::Budget,
};
use std::{ops::ControlFlow, sync::mpsc, time::Duration};

fn catalog() -> Catalog {
    Catalog::parse(br#"{"models":[{"slug":"gpt-5.6-luna","visibility":"list","supported_reasoning_levels":[{"effort":"medium"}]},{"slug":"gpt-5.6-terra","visibility":"list","supported_reasoning_levels":[{"effort":"medium"}]}]}"#).unwrap()
}
struct Backend {
    requests: mpsc::Sender<String>,
    generation_release: Option<mpsc::Receiver<()>>,
    catalogs: usize,
    catalog_started: mpsc::Sender<()>,
    catalog_release: mpsc::Receiver<()>,
}
impl TestBackend for Backend {
    fn login(
        &mut self,
        _: &Budget<'_>,
        _: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), client::Error> {
        Ok(())
    }
    fn catalog(&mut self, budget: &Budget<'_>) -> Result<Option<Catalog>, client::Error> {
        self.catalogs += 1;
        if self.catalogs > 1 {
            self.catalog_started.send(()).unwrap();
            self.catalog_release
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
        budget.check()?;
        Ok(Some(catalog()))
    }
    fn generate(
        &mut self,
        request: &Request,
        budget: &Budget<'_>,
        _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error> {
        self.requests
            .send(request.encode(2 * 1024 * 1024)?)
            .unwrap();
        if let Some(release) = self.generation_release.take() {
            release.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        budget.check()?;
        Ok(session::tests::response("done", Status::Completed))
    }
}
struct Fixture {
    model: model::Model,
    session: Session,
    requests: mpsc::Receiver<String>,
    generation_release: mpsc::Sender<()>,
    catalog_started: mpsc::Receiver<()>,
    catalog_release: mpsc::Sender<()>,
}
impl Fixture {
    fn new() -> Self {
        let (requests_tx, requests) = mpsc::channel();
        let (generation_release, generation_rx) = mpsc::channel();
        let (catalog_tx, catalog_started) = mpsc::channel();
        let (catalog_release, catalog_rx) = mpsc::channel();
        let backend = Backend {
            requests: requests_tx,
            generation_release: Some(generation_rx),
            catalogs: 0,
            catalog_started: catalog_tx,
            catalog_release: catalog_rx,
        };
        let mut fixture = Self {
            model: account::model(session::Model::Luna, None),
            session: Session::with_backend(session::Model::Luna, backend, None).unwrap(),
            requests,
            generation_release,
            catalog_started,
            catalog_release,
        };
        fixture.until_ready();
        fixture
    }
    fn key(&mut self, key: Key) {
        account::input(&mut self.model, key, &mut self.session);
    }
    fn event(&mut self, event: Event) {
        let ended = match &event {
            Event::Finished(end, _) => Some(*end),
            _ => None,
        };
        account::event(&mut self.model, event);
        commands::complete_argument(&mut self.model, &mut self.session);
        if let Some(end) = ended {
            account::after_finished(&mut self.model, &mut self.session, end);
        }
        account::drain_queue(&mut self.model, &mut self.session);
    }
    fn until(&mut self, done: impl Fn(&model::Model, &Session) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done(&self.model, &self.session) {
            if let Some(event) = self.session.poll() {
                self.event(event);
            }
            assert!(
                Instant::now() < deadline,
                "session did not reach expected state"
            );
            std::thread::yield_now();
        }
    }
    fn until_ready(&mut self) {
        self.until(|model, session| session.ready() && model.account.as_ref().unwrap().ready());
    }
}

#[test]
fn refreshed_model_command_waits_for_ready_and_preserves_a_new_draft() {
    let mut fixture = Fixture::new();
    fixture.model.account.as_mut().unwrap().catalog = None;
    fixture.key(Key::Text("/model terra".into()));
    fixture.key(Key::Enter);
    fixture
        .catalog_started
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    fixture.key(Key::Text("keep my newly typed task".into()));
    fixture.catalog_release.send(()).unwrap();
    fixture.until_ready();
    assert_eq!(
        fixture.model.account.as_ref().unwrap().selected,
        session::Model::Terra
    );
    assert_eq!(fixture.model.editor.text, "keep my newly typed task");
    assert!(fixture.model.command_receipts.values().any(
        |receipt| receipt.input == "/model terra" && receipt.status == lab::model::Status::Done
    ));
    assert!(fixture.requests.try_recv().is_err());
}

#[test]
fn stale_catalog_refresh_applies_an_effort_argument_after_ready() {
    let mut fixture = Fixture::new();
    fixture.model.account.as_mut().unwrap().catalog = Some(catalog().stale_for_test());
    fixture.key(Key::Text("/effort default".into()));
    fixture.key(Key::Enter);
    fixture
        .catalog_started
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    fixture.catalog_release.send(()).unwrap();
    fixture.until_ready();
    assert_eq!(
        fixture.model.account.as_ref().unwrap().selected.effort(),
        None
    );
    assert!(
        fixture
            .model
            .command_receipts
            .values()
            .any(|receipt| receipt.input == "/effort default"
                && receipt.status == lab::model::Status::Done)
    );
    assert!(fixture.requests.try_recv().is_err());
}

#[test]
fn cancelled_refresh_keeps_the_selection_and_the_current_draft() {
    let mut fixture = Fixture::new();
    fixture.model.account.as_mut().unwrap().catalog = None;
    fixture.key(Key::Text("/model terra".into()));
    fixture.key(Key::Enter);
    fixture
        .catalog_started
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    fixture.key(Key::Text("draft after command".into()));
    fixture.session.cancel();
    fixture.catalog_release.send(()).unwrap();
    fixture.until_ready();
    assert_eq!(
        fixture.model.account.as_ref().unwrap().selected,
        session::Model::Luna
    );
    assert_eq!(fixture.model.editor.text, "draft after command");
    assert!(
        fixture
            .model
            .command_receipts
            .values()
            .any(|receipt| receipt.input == "/model terra"
                && receipt.status == lab::model::Status::Warned)
    );
}

#[test]
fn bare_effort_waits_for_refresh_then_opens_the_picker_without_touching_the_draft() {
    let mut fixture = Fixture::new();
    fixture.model.account.as_mut().unwrap().catalog = None;
    fixture.key(Key::Text("/effort".into()));
    fixture.key(Key::Enter);
    fixture
        .catalog_started
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    fixture.key(Key::Text("keep editing".into()));
    fixture.catalog_release.send(()).unwrap();
    fixture.until_ready();
    assert_eq!(fixture.model.editor.text, "keep editing");
    assert!(fixture.model.menu.panel.is_some());
    assert!(fixture.requests.try_recv().is_err());
}

#[test]
fn queued_status_is_local_and_following_work_keeps_fifo_order() {
    let mut fixture = Fixture::new();
    fixture.key(Key::Text("original task".into()));
    fixture.key(Key::Enter);
    fixture
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    fixture.key(Key::Text("/status".into()));
    fixture.key(Key::Enter);
    fixture.key(Key::Text("following task".into()));
    fixture.key(Key::Enter);
    fixture.key(Key::Text("current draft".into()));
    fixture.generation_release.send(()).unwrap();
    fixture.until_ready();
    assert!(
        fixture
            .model
            .command_receipts
            .values()
            .any(|receipt| receipt.input == "/status")
    );
    assert_eq!(fixture.model.editor.text, "current draft");
    let prompts: Vec<_> = fixture
        .model
        .blocks
        .iter()
        .filter(|block| block.speaker == "You")
        .map(|block| block.text.as_str())
        .collect();
    assert_eq!(prompts, ["original task", "following task"]);
    assert!(
        fixture
            .requests
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .contains("following task")
    );
    assert!(fixture.requests.try_recv().is_err());
}

#[test]
fn queued_model_change_is_acknowledged_before_following_task() {
    let mut fixture = Fixture::new();
    fixture.key(Key::Text("original task".into()));
    fixture.key(Key::Enter);
    fixture
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    fixture.key(Key::Text("/model terra".into()));
    fixture.key(Key::Enter);
    fixture.key(Key::Text("task under terra".into()));
    fixture.key(Key::Enter);
    fixture.generation_release.send(()).unwrap();
    fixture.until_ready();
    assert_eq!(
        fixture.model.account.as_ref().unwrap().selected,
        session::Model::Terra
    );
    let request = fixture
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert!(
        request.contains("task under terra") && request.contains("gpt-5.6-terra"),
        "{request}"
    );
    assert!(
        fixture
            .model
            .blocks
            .iter()
            .all(|block| block.speaker != "You" || block.text != "/model terra")
    );
}

#[test]
fn dismissed_queued_picker_releases_the_next_item_without_sending_the_command() {
    let mut fixture = Fixture::new();
    fixture.key(Key::Text("original task".into()));
    fixture.key(Key::Enter);
    fixture
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    fixture.key(Key::Text("/model".into()));
    fixture.key(Key::Enter);
    fixture.key(Key::Text("next task".into()));
    fixture.key(Key::Enter);
    fixture.generation_release.send(()).unwrap();
    fixture.until(|model, _| model.menu.panel.is_some());
    assert_eq!(
        fixture.model.account.as_ref().unwrap().pending_messages(),
        ["next task"]
    );
    fixture.key(Key::Escape);
    fixture.until_ready();
    assert!(fixture.model.menu.panel.is_none());
    assert!(
        fixture
            .requests
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .contains("next task")
    );
    assert!(
        fixture
            .model
            .blocks
            .iter()
            .all(|block| block.speaker != "You" || block.text != "/model")
    );
}

#[test]
fn queued_clear_keeps_later_work_and_live_draft_for_navigation() {
    let mut fixture = Fixture::new();
    fixture.key(Key::Text("original task".into()));
    fixture.key(Key::Enter);
    fixture
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    fixture.key(Key::Text("/clear".into()));
    fixture.key(Key::Enter);
    fixture.key(Key::Text("task after clear".into()));
    fixture.key(Key::Enter);
    fixture.key(Key::Text("still editing".into()));
    fixture.generation_release.send(()).unwrap();
    fixture.until(|model, _| model.navigation.is_some());
    assert!(matches!(
        fixture.model.navigation,
        Some(navigation::Request::Clear)
    ));
    assert_eq!(
        fixture.model.account.as_ref().unwrap().pending_messages(),
        ["task after clear"]
    );
    assert_eq!(fixture.model.editor.text, "still editing");
    assert!(fixture.requests.try_recv().is_err());
    // The same transfer object used by navigation carries ownership once into
    // the fresh controller; only its Ready boundary can claim the next item.
    let pending = navigation::Pending::capture(&fixture.model).unwrap();
    let mut next_model = account::model(session::Model::Luna, None);
    pending.restore(&mut next_model);
    let mut next_session = session::tests::ready_fixture_with_catalog(catalog());
    account::event(&mut next_model, Event::Ready);
    account::drain_queue(&mut next_model, &mut next_session);
    assert!(
        next_model
            .account
            .as_ref()
            .unwrap()
            .pending_messages()
            .is_empty()
    );
    assert_eq!(next_model.editor.text, "still editing");
    assert!(
        next_model
            .blocks
            .iter()
            .any(|block| block.speaker == "You" && block.text == "task after clear")
    );
}
