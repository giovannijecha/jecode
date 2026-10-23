use super::*;
use std::time::Duration;

const MODEL: &str = "fixture/model";

fn endpoint(tag: &str) -> Endpoint {
    Endpoint {
        tag: tag.into(),
        provider: None,
        prompt: Some(1.0),
        completion: Some(2.0),
        cache_read: None,
        cache_write: None,
        discount: None,
        context: Some(100_000),
        output: None,
        quantization: None,
        tier: None,
        status: Some(0),
        uptime: None,
        latency: None,
        throughput: None,
        requests: None,
        parameters: Some(vec!["tools".into(), "reasoning".into()]),
    }
}

fn measured(tag: &str, throughput: f64, latency: f64) -> Endpoint {
    Endpoint {
        throughput: Some(throughput),
        latency: Some(latency),
        requests: Some(50),
        ..endpoint(tag)
    }
}

fn listing(list: Vec<Endpoint>) -> Endpoints {
    Endpoints {
        model: MODEL.into(),
        list,
        fetched: Instant::now(),
    }
}

fn typical() -> Endpoints {
    listing(vec![
        measured("a", 60.0, 800.0),
        measured("b", 40.0, 1200.0),
        measured("c", 100.0, 400.0),
    ])
}

fn router(routing: Routing, endpoints: Option<Endpoints>) -> Router {
    let mut router = Router::new(routing);
    if let Some(endpoints) = endpoints {
        router.offer(endpoints);
    }
    router
}

fn encoded(fields: &[(&str, Value)]) -> Vec<String> {
    fields
        .iter()
        .map(|(name, value)| format!("{name}={}", value.encode()))
        .collect()
}

#[test]
fn routing_round_trips_and_rejects_unknown_or_invalid_values() {
    for routing in [
        Routing::Auto,
        Routing::Default,
        Routing::Endpoint("deepinfra/fp8".into()),
    ] {
        assert_eq!(Routing::parse(&routing.value()), Some(routing));
    }
    for text in [
        r#"{"mode":"price"}"#,
        r#"{"mode":"endpoint"}"#,
        r#"{"mode":"endpoint","endpoint":"has space"}"#,
        r#"{"endpoint":"a"}"#,
        r#""auto""#,
    ] {
        assert_eq!(Routing::parse(&crate::json::parse(text).unwrap()), None);
    }
    assert!(!valid_tag(&"x".repeat(129)));
    assert!(valid_tag("google-vertex/global"));
}

#[test]
fn the_guard_follows_medians_of_comparable_measured_endpoints_only() {
    let mut endpoints = typical();
    endpoints.list.extend([
        // None of these may move the medians: a premium tier, a degraded
        // endpoint, one without tools, sparse statistics and no statistics.
        Endpoint {
            tier: Some("priority".into()),
            ..measured("fast/priority", 900.0, 50.0)
        },
        Endpoint {
            status: Some(-1),
            ..measured("down", 1.0, 9000.0)
        },
        Endpoint {
            parameters: Some(vec![]),
            ..measured("toolless", 1.0, 9000.0)
        },
        Endpoint {
            requests: Some(policy::MIN_REQUESTS - 1),
            ..measured("sparse", 1.0, 9000.0)
        },
        endpoint("unmeasured"),
    ]);
    let guard = guard(&endpoints, false).unwrap();
    assert_eq!(
        guard,
        Guard {
            throughput: 30.0,
            latency: 1.6,
            measured: 3
        }
    );
}

#[test]
fn missing_statistics_never_count_as_fast_and_too_few_give_no_guard() {
    let mut endpoints = listing(vec![
        measured("a", 60.0, 800.0),
        measured("b", 40.0, 1200.0),
    ]);
    endpoints
        .list
        .extend((0..5).map(|index| endpoint(&format!("null/{index}"))));
    endpoints.list.push(Endpoint {
        latency: None,
        ..measured("half", 500.0, 1.0)
    });
    assert_eq!(guard(&endpoints, false), None);
    // Reasoning conversations count only endpoints that accept reasoning.
    let mut endpoints = typical();
    endpoints.list[2].parameters = Some(vec!["tools".into()]);
    assert!(guard(&endpoints, false).is_some());
    assert_eq!(guard(&endpoints, true), None);
}

#[test]
fn slow_bargains_fall_behind_while_similar_and_premium_endpoints_stay_eligible() {
    let endpoints = listing(vec![
        Endpoint {
            prompt: Some(0.1),
            ..measured("cheap-slow", 20.0, 3000.0)
        },
        measured("typical-a", 55.0, 900.0),
        Endpoint {
            prompt: Some(0.6),
            ..measured("typical-b", 60.0, 800.0)
        },
        Endpoint {
            prompt: Some(4.0),
            ..measured("premium", 200.0, 300.0)
        },
    ]);
    let guard = guard(&endpoints, false).unwrap();
    assert_eq!((guard.throughput, guard.latency), (28.8, 1.7));
    // A large responsiveness loss is moved behind; native price weighting
    // still chooses among the rest, so the premium endpoint is not forced.
    assert!(20.0 < guard.throughput && 3.0 > guard.latency);
    for endpoint in &endpoints.list[1..] {
        assert!(endpoint.throughput.unwrap() >= guard.throughput);
        assert!(endpoint.latency.unwrap() / 1000.0 <= guard.latency);
    }
}

#[test]
fn auto_keeps_native_routing_and_adds_preferences_only_from_fresh_data() {
    let mut auto = router(Routing::Auto, None);
    let now = Instant::now();
    assert!(auto.fields(MODEL, false, now).is_empty());
    auto.session = Some("conversation-1".into());
    assert_eq!(
        encoded(&auto.fields(MODEL, false, now)),
        [r#"session_id="conversation-1""#]
    );
    auto.offer(typical());
    let fields = encoded(&auto.fields(MODEL, false, now));
    assert_eq!(
        fields,
        [
            r#"session_id="conversation-1""#,
            r#"provider={"preferred_max_latency":1.6,"preferred_min_throughput":30}"#
        ]
    );
    for forbidden in [
        "sort",
        "order",
        "allow_fallbacks",
        "require_parameters",
        "only",
    ] {
        assert!(!fields.join(" ").contains(forbidden));
    }
    let stale = auto.endpoints(MODEL).unwrap().fetched + policy::FRESH;
    assert_eq!(auto.fields(MODEL, false, stale).len(), 1);
    assert_eq!(auto.fields("other/model", false, now).len(), 1);
    assert!(explain(&Routing::Auto, auto.guard(MODEL, false, stale)).contains("too little"));
    assert!(explain(&Routing::Auto, auto.guard(MODEL, false, now)).contains("(provisional)"));
    assert!(
        router(Routing::Default, Some(typical()))
            .fields(MODEL, false, now)
            .is_empty()
    );
}

#[test]
fn a_pin_is_exact_without_fallbacks_or_required_parameters() {
    let now = Instant::now();
    let pin = Routing::Endpoint("deepinfra/fp8".into());
    assert_eq!(
        encoded(&router(pin.clone(), None).fields(MODEL, false, now)),
        [r#"provider={"allow_fallbacks":false,"order":["deepinfra/fp8"]}"#]
    );
    let endpoints = listing(vec![
        endpoint("deepinfra/fp8"),
        endpoint("deepinfra/fp8/us"),
        endpoint("deepinfra/fp8x"),
        endpoint("deepinfra"),
    ]);
    let mut pinned = router(pin, Some(endpoints));
    pinned.session = Some("conversation-1".into());
    assert_eq!(
        encoded(&pinned.fields(MODEL, false, now)),
        [
            r#"provider={"allow_fallbacks":false,"ignore":["deepinfra/fp8/us"],"order":["deepinfra/fp8"]}"#
        ]
    );
}

#[test]
fn pinned_endpoint_limits_narrow_the_model_only_when_known() {
    let model = Some(Limits {
        context: 200_000,
        output: Some(64_000),
    });
    let mut listed = endpoint("one");
    listed.output = Some(16_000);
    let mut unlisted = endpoint("two");
    unlisted.context = None;
    let endpoints = listing(vec![listed, unlisted]);
    let one = router(Routing::Endpoint("one".into()), Some(endpoints.clone()));
    let limits = one.limits(MODEL, model).unwrap();
    assert_eq!((limits.context, limits.output), (100_000, Some(16_000)));
    let two = router(Routing::Endpoint("two".into()), Some(endpoints.clone()));
    let limits = two.limits(MODEL, model).unwrap();
    assert_eq!((limits.context, limits.output), (200_000, Some(64_000)));
    let limits = one.limits(MODEL, None).unwrap();
    assert_eq!((limits.context, limits.output), (100_000, Some(16_000)));
    let auto = router(Routing::Auto, Some(endpoints));
    assert_eq!(auto.limits(MODEL, model).unwrap().context, 200_000);
    assert_eq!(
        auto.limits("other/model", None)
            .map(|limits| limits.context),
        None
    );
}

#[test]
fn refreshes_happen_once_per_window_and_failures_wait_before_retrying() {
    let start = Instant::now();
    assert!(!router(Routing::Default, None).needs_refresh(MODEL, start));
    let mut auto = router(Routing::Auto, None);
    assert!(auto.needs_refresh(MODEL, start));
    auto.failed(MODEL, start);
    assert!(!auto.needs_refresh(MODEL, start + policy::RETRY - Duration::from_secs(1)));
    assert!(auto.needs_refresh(MODEL, start + policy::RETRY));
    assert!(auto.needs_refresh("other/model", start));
    let endpoints = listing(vec![]);
    let fetched = endpoints.fetched;
    auto.offer(endpoints);
    assert!(!auto.needs_refresh(MODEL, fetched + policy::FRESH - Duration::from_secs(1)));
    assert!(auto.needs_refresh(MODEL, fetched + policy::FRESH + policy::RETRY));
    // An older listing for the same model never replaces a newer one.
    let mut older = listing(vec![endpoint("old")]);
    older.fetched = fetched;
    let mut newer = listing(vec![endpoint("new")]);
    newer.fetched = fetched + Duration::from_secs(1);
    auto.offer(newer);
    auto.offer(older);
    assert!(auto.endpoints(MODEL).unwrap().find("new").is_some());
    assert!(router(Routing::Endpoint("p".into()), None).needs_refresh(MODEL, start));
}

#[test]
fn descriptions_keep_unknown_values_visible() {
    let full = Endpoint {
        prompt: Some(0.177),
        completion: Some(0.355),
        throughput: Some(52.4),
        latency: Some(900.0),
        discount: Some(0.89),
        quantization: Some("fp8".into()),
        cache_read: Some(0.0147),
        context: Some(1_048_576),
        ..endpoint("one")
    };
    assert_eq!(
        describe(&full, false),
        "$0.177/$0.355 · 52 tok/s · 0.9 s · 89% off · fp8 · cache $0.0147 · 1M ctx"
    );
    let unknown = Endpoint {
        prompt: None,
        completion: None,
        context: None,
        status: Some(-2),
        uptime: Some(91.4),
        tier: Some("flex".into()),
        parameters: Some(vec![]),
        ..endpoint("two")
    };
    assert_eq!(
        describe(&unknown, false),
        "no tools · price ? · no stats · status -2 · up 91% · flex"
    );
    let free = Endpoint {
        prompt: Some(0.0),
        completion: None,
        context: Some(163_840),
        parameters: Some(vec!["tools".into()]),
        ..endpoint("three")
    };
    assert_eq!(
        describe(&free, true),
        "no reasoning · $0/? · no stats · 164K ctx"
    );
    assert_eq!(price(Some(12.5)), "$12.5");
    assert_eq!(price(Some(150.0)), "$150");
    assert_eq!(quantity(12_345), "12.3K");
}

#[test]
fn entries_offer_auto_then_endpoints_and_keep_the_current_choice() {
    let mut endpoints = typical();
    endpoints.list.insert(
        0,
        Endpoint {
            parameters: Some(vec![]),
            ..endpoint("toolless")
        },
    );
    let pin = Routing::Endpoint("b".into());
    let list = entries(Some(&endpoints), &pin, false);
    let names: Vec<_> = list.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, ["Auto — Best balance", "a", "b", "c", "toolless"]);
    assert!(list[0].help.contains("prefers ≥ 30 tok/s"));
    assert!(list[2].description.starts_with("current · $1/$2"));
    assert!(list[2].help.starts_with("Pinned to b without fallbacks"));
    assert!(
        list.iter()
            .filter(|entry| entry.description.starts_with("current"))
            .count()
            == 1
    );
    assert!(list[4].problem.as_ref().unwrap().contains("tool calls"));
    assert!(list[..4].iter().all(|entry| entry.problem.is_none()));

    let missing = Routing::Endpoint("gone".into());
    let list = entries(Some(&endpoints), &missing, false);
    assert_eq!(list.last().unwrap().name, "gone");
    assert_eq!(
        list.last().unwrap().description,
        "current · not in the latest listing"
    );
    let list = entries(None, &missing, false);
    assert_eq!(list.len(), 2);
    assert!(list[0].help.contains("too little fresh performance data"));
    assert_eq!(list[1].description, "current · listing unavailable");

    let list = entries(None, &Routing::Default, false);
    assert_eq!(list[1].routing, Routing::Default);
    assert_eq!(list[1].description, "current · no routing preferences");
    assert_eq!(entries(None, &Routing::Auto, false).len(), 1);
    assert!(
        entries(None, &Routing::Auto, false)[0]
            .description
            .starts_with("current · ")
    );
}

#[test]
fn routing_entries_block_missing_reasoning_when_the_effort_requires_it() {
    let endpoints = listing(vec![Endpoint {
        parameters: Some(vec!["tools".into()]),
        ..endpoint("tools-only")
    }]);
    let entries_with_reasoning = entries(Some(&endpoints), &Routing::Auto, true);
    assert!(
        entries_with_reasoning[1]
            .problem
            .as_deref()
            .is_some_and(|problem| problem.contains("reasoning"))
    );
    assert!(
        entries(Some(&endpoints), &Routing::Auto, false)[1]
            .problem
            .is_none()
    );
}

#[test]
fn routing_pins_require_a_fresh_exact_and_compatible_listing_before_a_turn() {
    let endpoints = listing(vec![endpoint("one"), endpoint("one/eu")]);
    let now = endpoints.fetched;
    let one = router(Routing::Endpoint("one".into()), Some(endpoints.clone()));
    assert!(one.validate_pin(MODEL, false, now).is_ok());
    assert!(one.validate_pin(MODEL, false, now + policy::FRESH).is_err());
    assert!(one.validate_pin("other/model", false, now).is_err());
    assert!(
        router(Routing::Endpoint("one".into()), None)
            .validate_pin(MODEL, false, now)
            .is_err()
    );
    assert!(
        router(Routing::Endpoint("gone".into()), Some(endpoints.clone()))
            .validate_pin(MODEL, false, now)
            .is_err()
    );
    let mut no_reasoning = endpoints;
    no_reasoning.list[0].parameters = Some(vec!["tools".into()]);
    let pinned = router(Routing::Endpoint("one".into()), Some(no_reasoning));
    assert!(pinned.validate_pin(MODEL, true, now).is_err());
    assert!(pinned.validate_pin(MODEL, false, now).is_ok());
    assert!(
        router(Routing::Auto, None)
            .validate_pin(MODEL, true, now)
            .is_ok()
    );
    assert!(
        router(Routing::Default, None)
            .validate_pin(MODEL, true, now)
            .is_ok()
    );
}
