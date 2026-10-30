use super::*;

pub(in crate::tui) fn node_failure() -> Value {
    Value::object([
        ("exit_code", Value::number(126)),
        ("status", Value::string("exited")),
        ("stdout", Value::string("PASS earlier checks\n")),
        (
            "stderr",
            Value::string(concat!(
                "node:internal/modules/run_main:107\r\n",
                "    triggerUncaughtException(\r\n",
                "    ^\r\n\r\n",
                "locator.click: Timeout 10000ms exceeded.\r\n",
                "Call log:\r\n",
                "\x1b[2m  - waiting for locator('.item-open')\x1b[22m\r\n\r\n",
                "    at review.mjs:48:78 {\r\n",
                "  name: 'TimeoutError'\r\n}\r\n\r\n",
                "Node.js v24.18.0\r\n",
            )),
        ),
    ])
}

#[test]
fn compact_node_failure_leads_with_the_timeout_instead_of_runtime_boilerplate() {
    let arguments = Value::object([
        ("action", Value::string("poll")),
        ("process_id", Value::string("p1")),
    ]);
    let result = node_failure();
    for columns in [40, 80] {
        let shown = Presentation::default();
        let rendered = card(
            Tool {
                name: "process",
                arguments: &arguments,
                result: Some(&result),
                summary: "exit 126",
                change: None,
                presentation: &shown,
            },
            columns,
            false,
        );
        let plain = rendered
            .rows
            .iter()
            .map(Line::plain)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(plain.contains("Exit 126 · locator.click:"), "{plain}");
        assert!(
            plain.contains("Timeout") && plain.contains("10000ms"),
            "{plain}"
        );
        assert!(
            !plain.contains("node:internal") && !plain.contains("triggerUncaughtException"),
            "{plain}"
        );
        assert!(!plain.contains("[ESC]"), "{plain}");
        assert!(!plain.contains("PASS earlier"), "{plain}");
        if columns == 80 {
            assert!(plain.contains("review.mjs:48:78"), "{plain}");
        }
        assert!(
            rendered
                .rows
                .iter()
                .all(|row| text::cells(&row.plain()) <= columns)
        );
        assert!(rendered.hidden);
    }
}

#[test]
fn expanded_node_failure_keeps_the_complete_retained_diagnostic_and_exit_code() {
    let arguments = Value::object([
        ("action", Value::string("poll")),
        ("process_id", Value::string("p1")),
    ]);
    let result = node_failure();
    let original = result.clone();
    let shown = Presentation {
        expanded: true,
        ..Presentation::default()
    };
    let rendered = card(
        Tool {
            name: "process",
            arguments: &arguments,
            result: Some(&result),
            summary: "exit 126",
            change: None,
            presentation: &shown,
        },
        80,
        false,
    );
    let plain = rendered
        .rows
        .iter()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    for part in [
        "node:internal/modules/run_main:107",
        "triggerUncaughtException(",
        "locator.click: Timeout",
        "waiting for locator",
        "review.mjs:48:78",
        "Node.js v24.18.0",
        "PASS earlier checks",
        "exit 126",
    ] {
        assert!(plain.contains(part), "{part}: {plain}");
    }
    assert_eq!(result, original);
}

#[test]
fn node_headline_falls_back_for_incomplete_preambles_and_preserves_plain_errors() {
    for stderr in [
        "ordinary failure\nsecond line\n",
        "node:internal/modules/run_main:107\n    triggerUncaughtException(\n    ^\n",
        "node:internal/modules/run_main:107\nnot an exception preamble\n",
    ] {
        let result = Value::object([
            ("stderr", Value::string(stderr)),
            ("stdout", Value::string("PASS earlier checks\n")),
        ]);
        let entries = streams::entries(&result, true);
        assert_eq!(streams::headline(&entries, true), Some(0));
    }
}
