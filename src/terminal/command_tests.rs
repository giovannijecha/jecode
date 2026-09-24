use super::*;
use crate::{
    session::{Event, Model as Selected, command_tests as fixture},
    terminal::{
        Key, account,
        render::Renderer,
        style::Tone,
        view::{self, Layout},
        vt,
    },
};

#[test]
fn real_direct_command_streams_in_one_panel_and_keeps_the_draft() {
    let mut run = fixture::start("write", false);
    let mut model = account::model(Selected::Luna, Some(&run.files.0));
    account::event(&mut model, Event::Ready);
    model.editor.insert("run fixture");
    account::input(&mut model, Key::Enter, &mut run.session);
    model.editor.insert("next draft");
    let mut layout = Layout::default();
    let mut renderer = Renderer::default();
    let mut terminal = vt::Screen::new(120, 35);
    let mut planned = false;
    let mut output_seen = false;
    loop {
        let event = fixture::next(&mut run.session);
        planned |= matches!(event, Event::CommandPlanned { .. });
        output_seen |= matches!(event, Event::CommandOutput { .. });
        let done = matches!(event, Event::Finished(..));
        account::event(&mut model, event);
        for size in [(120, 35), (40, 18), (120, 35)] {
            terminal.resize(size.0, size.1);
            let frame = layout.frame(&model, size.0, size.1);
            terminal.feed(&renderer.draw(frame, size, false));
        }
        if done {
            break;
        }
    }
    assert!(planned && output_seen);
    assert!(run.files.0.join("command-result.txt").exists());
    assert_eq!(model.editor.text, "next draft");
    assert!(model.account.as_ref().unwrap().command.is_none());
    let text = terminal.text();
    assert_eq!(text.matches("Run command").count(), 1, "{text}");
    assert_eq!(text.matches("next draft").count(), 1, "{text}");
    assert!(!text.contains("Enter confirm"));
    assert!(text.contains("Command finished"));
}

#[test]
fn process_text_cannot_forge_a_receipt_and_running_chrome_stays_bounded() {
    let mut model = account::model(Selected::Luna, None);
    planned(
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
    assert!(rows.iter().any(|row| row.text.contains("Stopping command")));
    finished(&mut model, 1, "Command interrupted".into(), false, true);
    let rows = view::frame(&model, 100, 35);
    assert!(!rows.iter().any(|row| row.text.contains("Stopping command")));
    assert!(
        rows.iter()
            .any(|r| r.tone == Tone::Error && r.text.contains("Command interrupted"))
    );
}

#[test]
fn command_display_distinguishes_tabs_from_spaces_and_backslash_sequences() {
    let mut model = account::model(Selected::Luna, None);
    planned(
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
    assert!(
        rows.iter()
            .any(|row| row.tone == Tone::Accent
                && row.text.contains("$ printf 'left\\tright \\\\t'"))
    );
    assert!(
        rows.iter()
            .any(|row| row.tone == Tone::Accent && row.text.contains("$ printf 'second line'"))
    );
    assert!(
        rows.iter()
            .any(|row| row.tone == Tone::Muted && row.text.contains("Command escapes:"))
    );
}
