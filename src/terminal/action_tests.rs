use super::{
    Key,
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
    assert!(model.action_demo.is_some());
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
fn direct_demo_actions_start_without_a_decision_and_keep_the_draft() {
    let now = Instant::now();
    for prompt in ["/edit", "/command", "/command-error"] {
        let mut model = start(prompt, now);
        model.editor.insert("draft-kept");
        let rows = view::frame(&model, 80, 24);
        assert!(rows.iter().any(|row| row.text.contains("draft-kept")));
        assert!(
            !rows
                .iter()
                .any(|row| row.text.contains("Enter confirm") || row.text.contains("Allow once"))
        );
        assert_eq!(model.editor.text, "draft-kept");
        for step in 1..=4 {
            model.tick(now + Duration::from_millis(650 * step));
        }
        assert!(!model.streaming());
        assert_eq!(model.editor.text, "draft-kept");
        assert!(transcript(&model).contains("Simulated"));
    }
}

#[test]
fn edit_display_retains_diff_and_outcome_with_and_without_color() {
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
    model.input(Key::Text("next question".into()), now);
    assert!(model.tick(now + Duration::from_millis(650)));
    assert!(!model.streaming());
    assert!(transcript(&model).contains("Simulated edit complete · +1 -1"));
    assert_eq!(
        transcript(&model).matches("Edit src/settings.rs").count(),
        1
    );
    assert_eq!(model.editor.text, "next question");
}

#[test]
fn command_output_arrives_in_order_and_failure_remains_distinct() {
    let now = Instant::now();
    for prompt in ["/command", "/command-error"] {
        let mut model = start(prompt, now);
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
        assert_eq!(before.matches("Simulated command complete").count(), 1);
    }
}

#[test]
fn cancellation_keeps_partial_demo_output() {
    let now = Instant::now();
    let mut model = start("/command", now);
    model.tick(now + Duration::from_millis(650));
    model.input(Key::Escape, now + Duration::from_millis(660));
    assert!(!model.streaming());
    let before = transcript(&model);
    assert!(before.contains("running 2 tests"));
    assert!(before.contains("Preview interrupted"));
    assert!(!model.tick(now + Duration::from_secs(10)));
    assert_eq!(transcript(&model), before);
}

#[test]
fn direct_action_chrome_stays_within_terminal_bounds() {
    let now = Instant::now();
    for prompt in ["/edit", "/command"] {
        let mut model = start(prompt, now);
        model.editor.insert(&"draft ".repeat(50));
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

#[test]
fn actions_keep_one_display_and_result_through_resize() {
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
        assert_eq!(text.matches("draft-kept").count(), 1, "{text}");
        assert!(!text.contains("Enter confirm"));
        assert!(!model.streaming());
        assert!(
            renderer
                .draw(layout.frame(&model, size.0, size.1), size, false)
                .is_empty()
        );
    }
}
