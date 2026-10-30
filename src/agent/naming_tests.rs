use super::naming::clean;
use crate::{
    agent::{
        Agent,
        backend_tests::fixture::{Account, Reply},
    },
    sessions::{Document, Handle},
    test_support::Directory,
    tools::Tools,
};
use std::time::{Duration, Instant};

#[test]
fn a_reply_becomes_one_clean_line() {
    assert_eq!(
        clean("\"Fix session resume.\"\nIt covers the parser.").as_deref(),
        Some("Fix session resume")
    );
    assert_eq!(
        clean("\n  # **Riprogettare la dashboard**  ").as_deref(),
        Some("Riprogettare la dashboard")
    );
    assert_eq!(clean("“”"), None);
    assert_eq!(clean(" \n "), None);
}

fn titled(handle: &Handle) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !handle.titled() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    handle.titled()
}

#[test]
fn the_first_request_names_the_session_once_and_the_title_is_saved() {
    let directory = Directory::new();
    let home = Directory::new();
    let account = Account::new("alpha", vec![Reply::text("Search index repair.")]);
    let mut agent = Agent::new(account.client(), Tools::new(directory.path()).unwrap());
    agent.enable_sessions(home.path()).unwrap();
    agent.name_sessions();
    agent
        .prepare_turn("Fix the empty-query path in session search")
        .unwrap();
    let handle = agent.sessions().unwrap();
    assert!(titled(&handle));
    let document = handle.snapshot();
    assert_eq!(document.title(), "Search index repair");
    let saved = Document::parse(&document.value()).unwrap();
    assert_eq!(saved.title.as_deref(), Some("Search index repair"));
    let request = account.requests.lock().unwrap()[0].encode();
    assert!(request.contains("Fix the empty-query path"), "{request}");

    handle.name("Another name").unwrap();
    assert_eq!(handle.snapshot().title(), "Search index repair");
}

#[test]
fn without_naming_the_title_stays_the_request() {
    let directory = Directory::new();
    let home = Directory::new();
    let account = Account::new("alpha", vec![]);
    let mut agent = Agent::new(account.client(), Tools::new(directory.path()).unwrap());
    agent.enable_sessions(home.path()).unwrap();
    agent.prepare_turn("Fix the empty-query path").unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let handle = agent.sessions().unwrap();
    assert!(!handle.titled());
    assert_eq!(handle.snapshot().title(), "Fix the empty-query path");
    assert!(account.requests.lock().unwrap().is_empty());
}
