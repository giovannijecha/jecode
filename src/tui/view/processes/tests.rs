use super::*;

fn process(status: &'static str) -> Process {
    Process {
        id: "process-1".into(),
        command: "printf catalog".into(),
        status,
        exit_code: None,
        error: None,
    }
}

fn plain(panel: &Panel, width: usize, height: usize) -> String {
    render(panel, width, height)
        .0
        .iter()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n")
}

fn logs(stream: usize) -> Logs {
    Logs {
        id: "process-1".into(),
        process: Some(process("running")),
        stream,
        views: Default::default(),
        interactive: false,
        input_mode: false,
    }
}

#[test]
fn one_row_stop_confirmation_keeps_its_question_visible() {
    let mut panel = Panel::new(vec![process("running")]);
    panel.confirm_stop = Some(process("running"));
    assert!(plain(&panel, 40, 1).contains("Stop this process?"));
}

#[test]
fn list_is_a_standard_menu_with_status_command_id_and_empty_message() {
    let active = plain(&Panel::new(vec![process("running")]), 80, 10);
    assert!(active.contains("Background processes · this conversation"));
    assert!(active.contains("1 running"), "{active}");
    assert!(active.contains("› running  printf catalog"), "{active}");
    assert!(active.contains("process-1"), "{active}");
    assert!(active.contains(" Ctrl+D  stop"), "{active}");
    let empty = plain(&Panel::new(vec![]), 80, 8);
    assert!(empty.contains("No active background processes in this conversation"));
    assert!(empty.contains(" Esc  close"));
}

#[test]
fn stop_confirmation_replaces_the_key_row_and_marks_the_target() {
    let mut panel = Panel::new(vec![process("running")]);
    panel.confirm_stop = Some(process("running"));
    let rows = render(&panel, 80, 10).0;
    let text = plain(&panel, 80, 10);
    assert!(text.contains("Stop this process?"), "{text}");
    assert!(text.contains(" Enter  stop"));
    assert!(text.contains(" Esc  cancel"));
    assert!(!text.contains("Ctrl+D"));
    let target = rows
        .iter()
        .find(|row| row.plain().contains("printf catalog"))
        .unwrap();
    assert!(
        target
            .spans
            .iter()
            .any(|span| span.text.contains("printf catalog") && span.style == BAD)
    );
    panel.logs = Some(logs(0));
    assert!(plain(&panel, 80, 10).contains("Stop this process?"));
    panel.confirm_stop = Some(process("exited"));
    let finished = plain(&panel, 80, 10);
    assert!(finished.contains("Already finished"));
    assert!(finished.contains(" Enter  back"));
}

#[test]
fn output_has_breadcrumb_status_stream_tabs_and_follow_mode() {
    let mut panel = Panel::new(vec![process("running")]);
    panel.logs = Some(logs(1));
    let view = plain(&panel, 80, 12);
    assert!(view.contains("Processes › printf catalog"), "{view}");
    assert!(view.contains("● running · process-1"), "{view}");
    assert!(view.contains("Output   Errors"), "{view}");
    assert!(view.contains("following"));
    assert!(view.contains("No output yet"));
    assert!(view.contains(" Tab  Output"));
    assert!(view.contains(" Esc  back"));
    let rows = render(&panel, 80, 12).0;
    let underline = rows.iter().find(|row| row.plain().contains('━')).unwrap();
    // The active section's underline sits under "Errors".
    let start = underline.plain().chars().position(|c| c == '━').unwrap();
    let names = rows
        .iter()
        .find(|row| row.plain().contains("Output   Errors"))
        .unwrap()
        .plain();
    assert_eq!(names.chars().position(|c| c == 'E'), Some(start));
}

#[test]
fn interactive_output_shows_input_entry_and_explicit_exit_keys() {
    let mut panel = Panel::new(vec![process("running")]);
    let mut log = logs(0);
    log.interactive = true;
    panel.logs = Some(log);
    let browsing = plain(&panel, 80, 14);
    assert!(browsing.contains(" i  input"), "{browsing}");

    panel.logs.as_mut().unwrap().input_mode = true;
    let typing = plain(&panel, 80, 14);
    assert!(typing.contains("Ctrl+]  back"), "{typing}");
    assert!(typing.contains("Ctrl+C  interrupt"), "{typing}");
    assert!(!typing.contains("Tab  Errors"), "{typing}");
}

#[test]
fn terminal_log_view_strips_vt_without_changing_saved_stream_bytes() {
    let mut panel = Panel::new(vec![process("running")]);
    let mut log = logs(0);
    log.interactive = true;
    let raw = "\x1b]0;secret title\x07hello\rDONE\x1b[31m\n";
    log.views[0].pages.push_back(crate::tools::processes::Page {
        text: raw.into(),
        start: 0,
        end: raw.len() as u64,
        length: raw.len() as u64,
    });
    panel.logs = Some(log);
    let rendered = plain(&panel, 80, 14);
    assert!(rendered.contains("DONEo"), "{rendered}");
    assert!(!rendered.contains("[ESC]"), "{rendered}");
    assert!(!rendered.contains("secret title"), "{rendered}");
    assert_eq!(panel.logs.as_ref().unwrap().views[0].text(), raw);
}

#[test]
fn paused_output_wraps_at_spaces_inside_the_inset() {
    let mut panel = Panel::new(vec![process("running")]);
    let mut view = logs(0);
    view.views[0]
        .pages
        .push_back(crate::tools::processes::Page {
            start: 0,
            text: "alpha beta gamma delta epsilon zeta eta theta iota kappa\n".into(),
            ..Default::default()
        });
    view.views[0].follow = false;
    panel.logs = Some(view);
    let text = plain(&panel, 30, 14);
    assert!(text.contains("paused"), "{text}");
    for word in ["alpha", "epsilon", "kappa"] {
        assert!(text.contains(word), "{word} split:\n{text}");
    }
}
