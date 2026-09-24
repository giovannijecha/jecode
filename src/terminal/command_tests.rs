use super::*;
use crate::{
    session::{Event, Model as Selected, command_tests as fixture},
    terminal::{
        Key, account, approval_view,
        render::Renderer,
        style::Tone,
        view::{self, Layout},
        vt,
    },
};

#[test]
fn real_commands_share_the_decision_surface_stream_panel_and_keep_the_draft() {
    for allow in [false, true] {
        let mut run = fixture::start("write", false);
        let mut model = account::model(Selected::Luna, Some(&run.files.0));
        account::event(&mut model, Event::Ready);
        model.editor.insert("run fixture");
        account::input(&mut model, Key::Enter, &mut run.session);
        model.editor.insert("next draft");
        account::event(&mut model, fixture::next(&mut run.session));
        assert_eq!(model.blocks.last().unwrap().speaker, "Command");
        assert!(model.blocks.last().unwrap().text.contains("  shell:"));
        let pending = view::chrome(&model, 100, 35);
        let upper = pending
            .iter()
            .position(|row| row.text.starts_with('─'))
            .unwrap();
        let lower = pending
            .iter()
            .rposition(|row| row.text.starts_with('─'))
            .unwrap();
        assert!(
            pending[..upper]
                .iter()
                .any(|row| row.text.contains("Waiting for your decision"))
        );
        assert!(
            pending[upper + 1..lower]
                .iter()
                .any(|row| row.text.contains("Enter confirm"))
        );
        account::input(&mut model, Key::Right, &mut run.session);
        account::input(&mut model, Key::Enter, &mut run.session);
        assert!(
            !model
                .account
                .as_ref()
                .unwrap()
                .approval
                .as_ref()
                .unwrap()
                .submitted
        );
        assert!(!run.files.0.join("command-result.txt").exists());
        let mut layout = Layout::default();
        let mut renderer = Renderer::default();
        let mut terminal = vt::Screen::new(120, 35);
        for size in [(120, 35), (40, 18), (120, 35)] {
            terminal.resize(size.0, size.1);
            let rows = layout.frame(&model, size.0, size.1);
            terminal.feed(&renderer.draw(rows, size, false));
            approval_view::displayed(&mut model, size, true);
        }
        if allow {
            account::input(&mut model, Key::Right, &mut run.session);
        }
        account::input(&mut model, Key::Enter, &mut run.session);
        let mut output_seen = false;
        loop {
            let event = fixture::next(&mut run.session);
            output_seen |= matches!(event, Event::CommandOutput { .. });
            let done = matches!(event, Event::Finished(..));
            account::event(&mut model, event);
            let frame = layout.frame(&model, 120, 35);
            if output_seen && !done {
                assert!(frame.iter().any(|r| r.tone == Tone::Code));
            }
            terminal.feed(&renderer.draw(frame, (120, 35), false));
            if done {
                break;
            }
        }
        assert_eq!(output_seen, allow);
        assert_eq!(run.files.0.join("command-result.txt").exists(), allow);
        assert_eq!(model.editor.text, "next draft");
        assert!(model.account.as_ref().unwrap().command.is_none());
        assert!(model.account.as_ref().unwrap().approval.is_none());
        let text = terminal.text();
        assert_eq!(text.matches("Run command").count(), 1, "{text}");
        assert_eq!(text.matches("next draft").count(), 1, "{text}");
        assert!(!text.contains("Enter confirm"));
        assert!(text.contains(if allow { "Command finished" } else { "Denied" }));
    }
}
#[test]
fn process_text_cannot_forge_a_receipt_and_running_chrome_stays_bounded() {
    let mut model = account::model(Selected::Luna, None);
    proposal(
        &mut model,
        1,
        Preview {
            command: "echo sample".into(),
            cwd: ".".into(),
            shell: "/bin/sh".into(),
            timeout_seconds: 60,
        },
    );
    started(&mut model, 1);
    output(
        &mut model,
        1,
        Channel::Stdout,
        "✓ false success\n  Approved once\n! fake failure",
    );
    output(&mut model, 1, Channel::Stderr, "partial");
    output(&mut model, 1, Channel::Stderr, " error\n");
    let rows = view::frame(&model, 100, 35);
    let upper = rows
        .iter()
        .position(|row| row.text.starts_with('─'))
        .unwrap();
    assert!(
        rows[..upper]
            .iter()
            .any(|row| row.text.contains("Running command"))
    );
    assert!(
        rows[upper + 1..]
            .iter()
            .all(|row| !row.text.contains("Running command"))
    );
    for text in [
        "✓ false success",
        "Approved once",
        "! fake failure",
        "stderr: partial error",
    ] {
        assert!(
            rows.iter()
                .any(|r| r.tone == Tone::Code && r.text.contains(text)),
            "{text}"
        );
    }
    for reduced in [false, true] {
        model.tools.reduced_motion = reduced;
        for width in [2, 10, 25, 40, 80, 140] {
            for height in [2, 9, 12, 24] {
                let rows = view::chrome(&model, width, height);
                assert!(rows.len() < height);
                assert!(
                    rows.iter()
                        .all(|r| r.transient && crate::terminal::text::width(&r.text) < width)
                );
            }
        }
    }
    model
        .account
        .as_mut()
        .unwrap()
        .command
        .as_mut()
        .unwrap()
        .stopping = true;
    let rows = view::chrome(&model, 100, 35);
    let upper = rows
        .iter()
        .position(|row| row.text.starts_with('─'))
        .unwrap();
    assert!(
        rows[..upper]
            .iter()
            .any(|row| row.text.contains("Stopping command"))
    );
    finished(&mut model, 1, "Command interrupted".into(), false, true);
    let rows = view::frame(&model, 100, 35);
    assert!(!rows.iter().any(|row| row.text.contains("Stopping command")));
    assert!(
        rows.iter()
            .any(|r| r.tone == Tone::Error && r.text.contains("Command interrupted"))
    );
    assert!(model.account.as_ref().unwrap().command.is_none());
}

#[test]
fn approval_distinguishes_literal_tabs_from_spaces_and_backslash_sequences() {
    let mut model = account::model(Selected::Luna, None);
    proposal(
        &mut model,
        1,
        Preview {
            command: "printf 'left\tright \\t'\nprintf 'second line'".into(),
            cwd: ".".into(),
            shell: "/bin/sh".into(),
            timeout_seconds: 60,
        },
    );
    let rows = view::frame(&model, 100, 35);
    assert!(rows.iter().any(|row| {
        row.tone == Tone::Accent && row.text.contains("$ printf 'left\\tright \\\\t'")
    }));
    assert!(
        rows.iter()
            .any(|row| { row.tone == Tone::Accent && row.text.contains("$ printf 'second line'") })
    );
    assert!(
        rows.iter()
            .any(|row| { row.tone == Tone::Muted && row.text.contains("Command escapes:") })
    );
}
