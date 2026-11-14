//! A live compact chapter keeps its height, and its rows keep whole facts,
//! names and badges at every width.

use super::*;

fn done() -> Value {
    Value::object([("exit_code", Value::number(0))])
}

#[test]
fn a_finishing_call_keeps_its_row_in_a_long_live_chapter() {
    let shown = Presentation::default();
    let args = object([("command", "run"), ("work", "Inspect")]);
    let ok = done();
    for columns in [40, 80, 120] {
        let mut calls: Vec<_> = (0..6)
            .map(|_| tool("bash", &args, Some(&ok), &shown))
            .collect();
        calls.push(tool("bash", &args, None, &shown));
        let live = chapter::header(&calls, columns, false).rows.len();
        calls[6] = tool("bash", &args, Some(&ok), &shown);
        let settled = chapter::header(&calls, columns, false).rows.len();
        assert_eq!(live, settled, "{columns} columns");
        calls.push(tool("bash", &args, None, &shown));
        let next = chapter::header(&calls, columns, false);
        assert_eq!(next.rows.len(), settled, "{columns} columns");
        // The running call takes the newest slot of the window, in call order.
        let text = lines(&next);
        assert!(text[text.len() - 2].contains("Running"), "{text:#?}");
    }
}

#[test]
fn running_never_opens_a_fact_row_of_its_own() {
    let shown = Presentation::default();
    let read = object([("path", "src/a.rs"), ("work", "Inspect")]);
    let bash = object([("command", "run"), ("work", "Inspect")]);
    let source = Value::object([("content", Value::string("1: source\n"))]);
    let ok = done();
    for columns in [24, 40, 80] {
        let settled = [
            tool("read", &read, Some(&source), &shown),
            tool("bash", &bash, Some(&ok), &shown),
            tool("read", &read, Some(&source), &shown),
        ];
        let live = [
            tool("read", &read, Some(&source), &shown),
            tool("bash", &bash, Some(&ok), &shown),
            tool("read", &read, None, &shown),
        ];
        let before = chapter::header(&live, columns, false).rows.len();
        let after = chapter::header(&settled, columns, false).rows.len();
        assert!(before <= after, "{columns} columns: {before} > {after}");
    }
}

#[test]
fn fact_rows_hold_whole_facts_and_end_two_cells_from_the_edge() {
    let shown = Presentation::default();
    let read = object([("path", "src/a.rs"), ("work", "Inspect")]);
    let bash = object([("command", "run"), ("work", "Inspect")]);
    let source = Value::object([("content", Value::string("1: source\n"))]);
    let background = object([("status", "running"), ("process_id", "p1")]);
    let stopped = Value::object([("cancelled", Value::Bool(true))]);
    let ok = done();
    let calls = [
        tool("read", &read, Some(&source), &shown),
        tool("bash", &bash, Some(&ok), &shown),
        tool("bash", &bash, Some(&background), &shown),
        tool("bash", &bash, Some(&stopped), &shown),
    ];
    for columns in [32, 40, 80] {
        let card = chapter::header(&calls, columns, false);
        let facts: Vec<_> = lines(&card)
            .into_iter()
            .skip(1)
            .take_while(|row| !row.contains("BASH") && !row.contains("READ"))
            .collect();
        assert!(!facts.is_empty());
        for row in &facts {
            assert!(
                text::cells(row.trim_end()) <= columns - 2,
                "{columns}: {row}"
            );
            let body = row.trim_start_matches("  │ ").trim_end_matches(" …");
            for fact in body.split(" · ") {
                let (count, noun) = fact.split_once(' ').unwrap_or((fact, ""));
                assert!(
                    count.parse::<usize>().is_ok() && !noun.is_empty(),
                    "{columns}: split fact {fact:?} in {row:?}"
                );
            }
        }
    }
}

#[test]
fn the_footer_counts_folded_calls_in_words() {
    let shown = Presentation::default();
    let args = object([("command", "run"), ("work", "Inspect")]);
    let ok = done();
    let two: Vec<_> = (0..2)
        .map(|_| tool("bash", &args, Some(&ok), &shown))
        .collect();
    let text = lines(&chapter::header(&two, 80, true)).join("\n");
    assert!(text.contains("└ no calls folded"), "{text}");
    let six: Vec<_> = (0..6)
        .map(|_| tool("bash", &args, Some(&ok), &shown))
        .collect();
    let text = lines(&chapter::header(&six, 80, true)).join("\n");
    assert!(text.contains("└ 2 calls folded"), "{text}");
    let five: Vec<_> = (0..5)
        .map(|_| tool("bash", &args, Some(&ok), &shown))
        .collect();
    let text = lines(&chapter::header(&five, 80, true)).join("\n");
    assert!(text.contains("└ 1 call folded"), "{text}");
}

#[test]
fn narrow_rows_drop_the_right_side_before_the_badge_or_name() {
    let shown = Presentation::default();
    let args = object([("command", "run"), ("work", "Inspect")]);
    let failed = Value::object([("exit_code", Value::number(1))]);
    let calls: Vec<_> = (0..33)
        .map(|index| {
            let result = (index != 32).then_some(&failed);
            tool("bash", &args, result, &shown)
        })
        .collect();
    for columns in [20, 24, 28] {
        let text = lines(&chapter::header(&calls, columns, false));
        assert!(text[0].contains("WORK"), "{columns}: {}", text[0]);
        assert!(
            text.iter().filter(|row| row.contains("BASH")).count() >= 2,
            "{columns}: {text:#?}"
        );
    }
}
