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

#[test]
fn one_row_stop_confirmation_keeps_its_action_visible() {
    let mut panel = Panel::new(vec![process("running")]);
    panel.confirm_stop = Some(process("running"));
    assert_eq!(plain(&panel, 40, 1), "  Stop process?");
}

#[test]
fn source_list_has_status_column_command_and_empty_message() {
    let active = plain(&Panel::new(vec![process("running")]), 80, 8);
    assert!(active.contains("Background processes"));
    assert!(active.contains("› running  printf catalog"), "{active}");
    assert!(active.contains("Ctrl+D stop · Esc close"), "{active}");
    let empty = plain(&Panel::new(vec![]), 80, 8);
    assert!(empty.contains("No active background processes in this conversation"));
    assert!(empty.contains("Esc close"));
}

#[test]
fn stop_confirmation_replaces_list_or_output_and_names_target() {
    let mut panel = Panel::new(vec![process("running")]);
    panel.confirm_stop = Some(process("running"));
    let confirmation = plain(&panel, 80, 8);
    assert!(confirmation.contains("Stop process?"));
    assert!(confirmation.contains("printf catalog"));
    assert!(confirmation.contains("running"));
    assert!(confirmation.contains("process-1"));
    assert!(confirmation.contains("Enter stop · Esc cancel"));
    assert!(!confirmation.contains("Background processes"));
    panel.logs = Some(Logs {
        id: "process-1".into(),
        process: Some(process("running")),
        stream: 0,
        views: Default::default(),
    });
    assert_eq!(plain(&panel, 80, 8), confirmation);
    panel.confirm_stop = Some(process("exited"));
    let finished = plain(&panel, 80, 8);
    assert!(finished.contains("Process finished"));
    assert!(finished.contains("Already finished · Enter back · Esc cancel"));
}

#[test]
fn output_header_shows_status_id_streams_and_follow_mode() {
    let mut panel = Panel::new(vec![process("running")]);
    panel.logs = Some(Logs {
        id: "process-1".into(),
        process: Some(process("running")),
        stream: 1,
        views: Default::default(),
    });
    let view = plain(&panel, 80, 10);
    assert!(view.contains("printf catalog"));
    assert!(view.contains("running · process-1"));
    assert!(view.contains("Output  Errors"));
    assert!(view.contains("following"));
    assert!(view.contains("No output yet"));
    let rows = render(&panel, 80, 10).0;
    let tabs = rows
        .iter()
        .find(|row| row.plain().contains("Output  Errors"))
        .unwrap();
    assert_eq!(
        tabs.spans
            .iter()
            .find(|span| span.text == "Errors")
            .unwrap()
            .style,
        ACCENT
    );
}
