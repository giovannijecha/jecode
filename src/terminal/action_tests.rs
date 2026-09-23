use super::{
    Key,
    input::Decoder,
    model::Model,
    render::Renderer,
    style::Tone,
    view::{self, Layout},
    vt,
};
use std::time::{Duration, Instant};

fn start(prompt: &str, now: Instant) -> Model {
    let mut model = Model::new(now);
    model.input(Key::Text(prompt.into()), now);
    model.input(Key::Enter, now);
    assert!(model.action_demo.as_ref().unwrap().pending());
    model
}
fn transcript(model: &Model) -> String {
    model
        .blocks
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn approval_waits_defaults_to_deny_and_does_not_consume_the_saved_draft() {
    let now = Instant::now();
    for prompt in ["/edit", "/command", "/command-error"] {
        for deny in [Key::Enter, Key::Escape, Key::Interrupt] {
            let mut model = start(prompt, now);
            model.editor.insert("draft-kept");
            let before = transcript(&model);
            assert!(!model.tick(now + Duration::from_secs(3600)));
            assert_eq!(transcript(&model), before);
            let mut decoder = Decoder::default();
            for key in decoder.push(b"\x1b[200~\x1b[C\ryes\x1b[201~", now) {
                model.input(key, now);
            }
            assert!(!model.action_demo.as_ref().unwrap().allow);
            assert_eq!(model.editor.text, "draft-kept");
            model.input(deny, now);
            assert!(!model.streaming());
            assert!(transcript(&model).contains("Denied"));
            assert!(!transcript(&model).contains("Approved once"));
            assert_eq!(model.editor.text, "draft-kept");
            assert!(!model.tick(now + Duration::from_secs(3601)));
        }
    }
}

#[test]
fn edit_preview_retains_diff_and_individual_outcome_with_and_without_color() {
    let now = Instant::now();
    let mut model = start("/edit", now);
    let before = view::frame(&model, 80, 24);
    for (prefix, tone) in [("-       2", Tone::Removed), ("+       3", Tone::Added)] {
        let row = before
            .iter()
            .find(|row| row.text.starts_with(prefix))
            .unwrap();
        assert_eq!(row.tone, tone);
        assert_eq!(row.paint(false), row.text);
        assert!(!row.paint(false).contains('\x1b'));
        assert!(row.paint(true).contains("48;2"));
    }
    model.input(Key::Right, now);
    model.input(Key::Enter, now);
    model.input(Key::Text("next question".into()), now);
    assert!(model.tick(now + Duration::from_millis(650)));
    assert!(!model.streaming());
    assert!(transcript(&model).contains("Simulated edit complete · +1 -1"));
    assert_eq!(
        transcript(&model).matches("Edit src/settings.rs").count(),
        1
    );
    assert_eq!(model.editor.text, "next question");
    assert!(!model.tick(now + Duration::from_secs(20)));
}

#[test]
fn command_output_arrives_in_order_and_failure_remains_distinct() {
    let now = Instant::now();
    for prompt in ["/command", "/command-error"] {
        let mut model = start(prompt, now);
        model.input(Key::Right, now);
        model.input(Key::Enter, now);
        let mut before = transcript(&model);
        assert!(!before.contains("running 2 tests"));
        for step in 1..=4 {
            assert!(model.tick(now + Duration::from_millis(650 * step)));
            let after = transcript(&model);
            assert!(after.starts_with(&before), "output was rewritten");
            before = after;
        }
        assert!(!model.streaming());
        let failed = prompt.ends_with("error");
        assert!(before.contains(if failed { "exit 101" } else { "exit 0" }));
        assert_eq!(before.contains("expected: 3, received: 2"), failed);
        assert_eq!(before.matches("$ cargo test --lib").count(), 1);
        assert_eq!(before.matches("Approved once").count(), 1);
        assert_eq!(before.matches("Simulated command complete").count(), 1);
        let rows = view::frame(&model, 100, 24);
        let summary = rows
            .iter()
            .find(|r| r.text.contains("Simulated command complete"))
            .unwrap();
        assert_eq!(
            summary.tone,
            if failed { Tone::Error } else { Tone::Heading }
        );
    }
}

#[test]
fn cancellation_keeps_output_and_decision_applies_only_to_one_action() {
    let now = Instant::now();
    let mut model = start("/command", now);
    model.input(Key::Right, now);
    model.input(Key::Enter, now);
    model.tick(now + Duration::from_millis(650));
    model.input(Key::Escape, now + Duration::from_millis(660));
    assert!(!model.streaming());
    let before = transcript(&model);
    assert!(before.contains("running 2 tests"));
    assert!(before.contains("Preview interrupted"));
    assert!(!model.tick(now + Duration::from_secs(10)));
    assert_eq!(transcript(&model), before);
    model.input(Key::Text("/edit".into()), now);
    model.input(Key::Enter, now);
    assert!(model.action_demo.as_ref().unwrap().pending());
    assert!(!model.action_demo.as_ref().unwrap().allow);
    model.input(Key::Quit, now);
    assert!(model.quit);
    assert!(!transcript(&model).contains("Simulated edit complete"));
}

#[test]
fn approval_and_running_chrome_stay_within_terminal_bounds() {
    let now = Instant::now();
    for prompt in ["/edit", "/command"] {
        let mut model = start(prompt, now);
        model.editor.insert(&"draft ".repeat(50));
        for running in [false, true] {
            if running {
                model.input(Key::Right, now);
                model.input(Key::Enter, now);
            }
            for width in [2, 10, 25, 40, 80, 140] {
                for height in [2, 9, 12, 24] {
                    let rows = view::chrome(&model, width, height);
                    assert!(rows.len() < height, "{width}x{height}: {} rows", rows.len());
                    for row in rows {
                        assert!(row.transient);
                        assert!(super::text::width(&row.text) < width, "{row:?}");
                        assert_eq!(
                            row.paint(false)
                                .replace("\x1b[7m", "")
                                .replace("\x1b[27m", ""),
                            row.text
                        );
                        row.paint(true);
                    }
                }
            }
        }
    }
}

#[test]
fn actions_keep_one_preview_and_result_through_resize() {
    let now = Instant::now();
    for prompt in ["/edit", "/command", "/command-error"] {
        let mut model = start(prompt, now);
        model.editor.insert("draft-kept");
        let mut layout = Layout::default();
        let mut renderer = Renderer::default();
        let mut terminal = vt::Screen::new(120, 30);
        let mut size = (120, 30);
        for n in 0..=200 {
            let at = now + Duration::from_millis(n * 20);
            if matches!(n, 10 | 30 | 60 | 120 | 180) {
                size = if matches!(n, 10 | 60) {
                    (45, 18)
                } else {
                    (120, 30)
                };
                terminal.resize(size.0, size.1);
            }
            if n == 20 {
                model.input(Key::Right, at);
                model.input(Key::Enter, at);
            }
            model.tick(at);
            terminal.feed(&renderer.draw(layout.frame(&model, size.0, size.1), size, false));
        }
        let text = terminal.text();
        let label = if prompt == "/edit" {
            "Edit src/settings.rs"
        } else {
            "$ cargo test --lib"
        };
        assert_eq!(text.matches(label).count(), 1, "{text}");
        assert_eq!(text.matches("Approved once").count(), 1, "{text}");
        assert_eq!(text.matches("draft-kept").count(), 1, "{text}");
        assert!(!text.contains("Enter confirm"), "approval archived: {text}");
        assert!(!model.streaming());
        assert!(
            renderer
                .draw(layout.frame(&model, size.0, size.1), size, false)
                .is_empty()
        );
    }
}
