//! Deterministic native sheets for checking the design catalog's synthetic views.
use crate::{
    backend::{Entry, Metrics, Routing},
    copy::Target,
    sessions::Summary,
    tui::{information::Information, selector::Selector, state::State, theme, view},
};

const MODEL: &str = "deepseek/deepseek-v4";
const DIRECTORY: &str = r"C:\Projects\example";

fn base(columns: usize) -> State {
    State {
        width: columns,
        height: 32,
        effort: "high".into(),
        ..State::default()
    }
}

fn selector(columns: usize, menu: Selector) -> State {
    let mut state = base(columns);
    state.selector = Some(menu);
    state
}

fn information(columns: usize, info: Information) -> State {
    let mut state = base(columns);
    state.information = Some(info);
    state
}

fn entries(current: &Routing, endpoints: bool) -> Vec<Entry> {
    let mut entries = vec![Entry {
        routing: Routing::Auto,
        name: Routing::Auto.label(),
        description: "native routing with fallbacks".into(),
        metrics: None,
        facts: String::new(),
        help: String::new(),
        problem: None,
    }];
    if endpoints {
        for (name, input, output, speed, latency) in [
            ("deepinfra/fp8", 0.216, 0.72, 48.0, 0.9),
            ("baidu/fp8", 0.27, 0.9, 61.0, 1.2),
        ] {
            entries.push(Entry {
                routing: Routing::Endpoint(name.into()),
                name: name.into(),
                description: name.into(),
                metrics: Some(Metrics {
                    input: Some(input),
                    output: Some(output),
                    throughput: Some(speed),
                    latency: Some(latency),
                }),
                facts: String::new(),
                help: String::new(),
                problem: None,
            });
        }
    } else if let Routing::Endpoint(name) = current {
        entries.push(Entry {
            routing: current.clone(),
            name: name.clone(),
            description: "listing unavailable".into(),
            metrics: None,
            facts: String::new(),
            help: String::new(),
            problem: None,
        });
    }
    entries
}

fn scenes(columns: usize) -> Vec<(&'static str, State)> {
    let auto = Routing::Auto;
    let pinned = Routing::Endpoint("baidu/fp8".into());
    let mut loading = Selector::loading("Routing", "Loading endpoints…", false);
    loading.context = MODEL.into();
    let now = crate::sessions::now();
    let sessions = [
        Summary {
            id: "current".into(),
            title: "Fix the session expiry check in auth".into(),
            updated: now,
            backend: "openrouter".into(),
            model: MODEL.into(),
        },
        Summary {
            id: "older".into(),
            title: "Add the sleep test".into(),
            updated: now - 86_400_000,
            backend: "openrouter".into(),
            model: MODEL.into(),
        },
    ];
    vec![
        ("routingLoading", selector(columns, loading)),
        (
            "routing",
            selector(
                columns,
                Selector::routing(MODEL, false, entries(&auto, true), &auto, vec![]),
            ),
        ),
        (
            "routingPinned",
            selector(
                columns,
                Selector::routing(MODEL, false, entries(&pinned, true), &pinned, vec![]),
            ),
        ),
        (
            "routingUnavailable",
            selector(
                columns,
                Selector::routing(
                    MODEL,
                    false,
                    entries(&pinned, false),
                    &pinned,
                    vec!["Endpoint listing unavailable · OpenRouter returned HTTP 503".into()],
                ),
            ),
        ),
        (
            "skillsInfo",
            information(
                columns,
                Information::new(
                    "Skills".into(),
                    vec![
                        (
                            "code-review".into(),
                            "Review changes for actionable defects".into(),
                        ),
                        ("debugging".into(), "Investigate an observed failure".into()),
                        ("Skipped".into(), "broken skill metadata".into()),
                    ],
                )
                .aside("2 available skills · 1 skipped"),
            ),
        ),
        (
            "tmpInfo",
            information(
                columns,
                Information::new(
                    "Temporary files".into(),
                    vec![
                        ("Path".into(), r"C:\Projects\example\.jecode\tmp".into()),
                        ("Files".into(), "3".into()),
                        ("Directories".into(), "1".into()),
                        ("Size".into(), "1280 bytes".into()),
                        (
                            "Retention".into(),
                            "Kept until /tmp clean, including after resume".into(),
                        ),
                    ],
                )
                .context("this session"),
            ),
        ),
        (
            "copy",
            selector(
                columns,
                Selector::copy(&[
                    Target {
                        name: "Last response".into(),
                        text: "The limiter now returns 429.".into(),
                    },
                    Target {
                        name: "Code block · rust".into(),
                        text: "const LOGIN_LIMIT: u32 = 10;".into(),
                    },
                ]),
            ),
        ),
        (
            "resume",
            selector(columns, Selector::sessions(&sessions, "current")),
        ),
    ]
}

#[test]
#[ignore = "prints current native sheets for catalog parity"]
fn catalog_sheets() {
    for (palette, light) in [("dark", false), ("light", true)] {
        theme::set_light(light);
        for columns in [44, 80, 120] {
            for (id, state) in scenes(columns) {
                println!("@@ {id} {columns} {palette}");
                for line in view::composer_frame(&state, MODEL, DIRECTORY).live {
                    println!("|{}", line.plain().trim_end());
                }
                println!("@@END");
            }
        }
    }
    theme::set_light(false);
}
