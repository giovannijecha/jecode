use super::*;
use crate::test_support::HttpFixture;

#[test]
fn routing_validation_masks_a_saved_key_used_as_an_endpoint_tag() {
    let key = "isolated-fixture-key";
    let fixture = HttpFixture::new(vec![(
        200,
        crate::json::parse(&format!(
            r#"{{"data":{{"endpoints":[{{"tag":"{key}","supported_parameters":[]}}]}}}}"#
        ))
        .unwrap(),
    )]);
    let services = crate::backend::Services::fixture(&fixture.endpoint);
    let error = endpoint_routing(&services, "fixture/model", Effort::Default, key)
        .expect_err("an endpoint without tools must be rejected");
    let Failure::Error(error) = error else {
        panic!("expected an endpoint error");
    };
    assert!(!error.contains(key), "{error}");
    assert!(error.contains("[redacted]"), "{error}");
    assert_eq!(fixture.finish().len(), 1);
}
