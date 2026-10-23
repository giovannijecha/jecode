use super::*;
use crate::backend::openrouter::routing::policy;

fn listing(endpoints: &str) -> Endpoints {
    let value =
        crate::json::parse(&format!(r#"{{"data":{{"endpoints":[{endpoints}]}}}}"#)).unwrap();
    parse("fixture/model", &value, Instant::now()).unwrap()
}

fn close(value: Option<f64>, expected: f64) -> bool {
    value.is_some_and(|value| (value - expected).abs() < 1e-9)
}

#[test]
fn listed_prices_already_include_the_discount_and_statistics_keep_their_units() {
    let endpoints = listing(
        r#"{"tag":"baidu/fp8","provider_name":"Baidu","context_length":163840,
        "max_completion_tokens":32768,"quantization":"fp8","status":0,"uptime_last_30m":99.5,
        "pricing":{"prompt":"0.0000003","completion":"0.0000012","input_cache_read":"0.00000003","discount":0.2},
        "perf_last_30m_by_workload":{"text_generation":{"latency":{"p50":850},"throughput":{"p50":62.5},"request_count":40}},
        "latency_last_30m":{"p50":0.25},"throughput_last_30m":{"p50":9},
        "supported_parameters":["tools","reasoning"]}"#,
    );
    let endpoint = &endpoints.list[0];
    assert_eq!(endpoint.tag, "baidu/fp8");
    assert_eq!(endpoint.provider.as_deref(), Some("Baidu"));
    // The listed $0.3/M is what is charged; the 20% discount is never applied again.
    assert!(close(endpoint.prompt, 0.3));
    assert!(close(endpoint.completion, 1.2));
    assert!(close(endpoint.cache_read, 0.03));
    assert_eq!(endpoint.cache_write, None);
    assert_eq!(endpoint.discount, Some(0.2));
    assert_eq!(
        (endpoint.context, endpoint.output),
        (Some(163840), Some(32768))
    );
    assert_eq!(endpoint.quantization.as_deref(), Some("fp8"));
    assert_eq!((endpoint.status, endpoint.uptime), (Some(0), Some(99.5)));
    // Workload statistics win over the legacy fields; latency is milliseconds.
    assert_eq!(endpoint.latency, Some(850.0));
    assert_eq!(endpoint.throughput, Some(62.5));
    assert_eq!(endpoint.requests, Some(40));
    assert_eq!(endpoint.supports("tools"), Some(true));
    assert_eq!(endpoint.supports("response_format"), Some(false));
}

#[test]
fn unknown_and_invalid_values_stay_distinct_from_zero() {
    let endpoints = listing(
        r#"{"tag":"free/one","pricing":{"prompt":"0","completion":"0"},"status":-2,
        "perf_last_30m_by_workload":null,"latency_last_30m":{"p50":0.25},"throughput_last_30m":{"p50":30}},
        {"tag":"variable/two","pricing":{"prompt":"-1","discount":1.5},"status":"busy",
        "quantization":"unknown","service_tier":"standard","uptime_last_30m":140,
        "perf_last_30m_by_workload":{"text_generation":{"latency":{"p50":0},"throughput":{"p50":null}}}},
        {"tag":"flex/three/flex","service_tier":"flex","pricing":{"prompt":"x"}}"#,
    );
    let [free, variable, flex] = &endpoints.list[..] else {
        panic!("three endpoints expected");
    };
    assert_eq!((free.prompt, free.completion), (Some(0.0), Some(0.0)));
    assert_eq!(free.status, Some(-2));
    // The legacy latency unit is ambiguous, so it is not used; throughput is.
    assert_eq!(free.latency, None);
    assert_eq!(free.throughput, Some(30.0));
    assert_eq!(free.parameters, None);
    assert_eq!(free.supports("tools"), None);
    assert_eq!((variable.prompt, variable.completion), (None, None));
    assert_eq!(variable.discount, None);
    assert_eq!(variable.status, None);
    assert_eq!(variable.uptime, None);
    assert_eq!((variable.latency, variable.throughput), (None, None));
    assert_eq!(
        (variable.quantization.as_ref(), variable.tier.as_ref()),
        (None, None)
    );
    assert_eq!(flex.tier.as_deref(), Some("flex"));
    assert_eq!(flex.prompt, None);
}

#[test]
fn invalid_tags_and_duplicates_are_skipped_and_a_missing_listing_fails() {
    let endpoints = listing(
        r#"{"tag":"one"},{"tag":"one","context_length":5},{"tag":"has space"},{"tag":""},{"name":"untagged"}"#,
    );
    assert_eq!(endpoints.list.len(), 1);
    assert_eq!(endpoints.list[0].context, None);
    assert!(endpoints.find("one").is_some());
    assert!(endpoints.find("one/extra").is_none());
    for body in [
        r#"{"data":{}}"#,
        r#"{"data":{"endpoints":{}}}"#,
        r#"{"error":"x"}"#,
    ] {
        let error = parse(
            "fixture/model",
            &crate::json::parse(body).unwrap(),
            Instant::now(),
        )
        .unwrap_err();
        assert_eq!(error, "OpenRouter returned an invalid endpoint listing");
    }
}

#[test]
fn listings_are_fresh_for_one_window() {
    let endpoints = listing("");
    assert!(endpoints.list.is_empty());
    assert!(endpoints.fresh(endpoints.fetched));
    assert!(endpoints.fresh(endpoints.fetched + policy::FRESH - std::time::Duration::from_secs(1)));
    assert!(!endpoints.fresh(endpoints.fetched + policy::FRESH));
}

#[test]
fn listing_requests_name_the_encoded_model_and_fail_without_an_author() {
    let fixture = crate::test_support::HttpFixture::new(vec![(
        200,
        crate::json::parse(r#"{"data":{"endpoints":[{"tag":"one"}]}}"#).unwrap(),
    )]);
    let api = Api::fixture(&fixture.endpoint);
    let endpoints = api.endpoints("fixture/model:free").unwrap();
    assert_eq!(endpoints.model, "fixture/model:free");
    assert_eq!(endpoints.list.len(), 1);
    let requests = fixture.finish();
    assert!(requests[0].headers[0].starts_with("GET /models/fixture/model%3Afree/endpoints "));
    assert!(api.endpoints("no-author").is_err());
}
