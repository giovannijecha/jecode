use super::*;
use crate::test_support::{Directory, HttpFixture};

#[test]
fn resume_keeps_saved_model_calibration_but_discards_direct_request_measurements() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let new_agent = || {
        let client = crate::openrouter::OpenRouter::fixture(fixture.endpoint.clone());
        Agent::new(client, crate::tools::Tools::new(directory.path()).unwrap())
    };
    let mut first = new_agent();
    first.enable_sessions(home.path()).unwrap();
    first.prepare_turn("Continue the same work").unwrap();
    first.context.observe(
        Some(&Value::object([("prompt_tokens", Value::number(1000))])),
        2,
    );
    first.context.calibrate(6000);
    first.save_session().unwrap();
    let id = first.sessions().unwrap().id();
    drop(first);
    let mut resumed = new_agent();
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert_eq!(resumed.context.calibration, Some((1000, 6000)));
    assert_eq!(resumed.context.input_tokens, None);
    assert_eq!(resumed.context.measured_end, 0);
    resumed.set_model("fixture/other".into()).unwrap();
    assert_eq!(resumed.context.calibration, None);
    assert!(fixture.finish().is_empty());
}
