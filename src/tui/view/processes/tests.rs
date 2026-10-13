use super::*;

fn process(status: &'static str) -> Process {
    Process {
        id: "process-1".into(),
        command: "printf secret-masked".into(),
        status,
        exit_code: None,
        error: None,
    }
}

fn visible(panel: &Panel, columns: usize, height: usize) -> String {
    render(panel, columns, height)
        .0
        .iter()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn empty_list_prioritizes_its_message_at_one_row() {
    let panel = Panel::new(vec![]);
    assert!(visible(&panel, 80, 8).contains("No active background processes in this conversation"));
    assert!(visible(&panel, 80, 1).contains("No active background processes"));
    assert_eq!(visible(&panel, 1, 1), "∅");
}

#[test]
fn stop_confirmation_shows_target_and_finished_transition() {
    let mut panel = Panel::new(vec![process("running")]);
    panel.confirm_stop = Some(process("running"));
    let active = visible(&panel, 80, 8);
    for detail in [
        "Stop process?",
        "printf secret-masked",
        "running",
        "process-1",
        "Enter stop · Esc cancel",
    ] {
        assert!(active.contains(detail), "{detail}\n{active}");
    }
    assert_eq!(visible(&panel, 1, 1), "?");
    assert!(visible(&panel, 10, 2).contains("↵"));
    let short = visible(&panel, 80, 2);
    assert!(short.contains("printf secret-masked"), "{short}");
    assert!(short.contains("Enter stop · Esc cancel"), "{short}");

    panel.confirm_stop = Some(process("exited"));
    let finished = visible(&panel, 80, 8);
    assert!(finished.contains("Process finished"), "{finished}");
    assert!(
        finished.contains("Already finished · Enter back"),
        "{finished}"
    );
    assert!(!finished.contains("Enter stop"), "{finished}");
    assert_eq!(visible(&panel, 1, 1), "✓");
    let short = visible(&panel, 80, 2);
    assert!(short.contains("printf secret-masked"), "{short}");
    assert!(short.contains("Enter back · Esc cancel"), "{short}");
}

#[test]
fn active_list_keeps_stop_hint_and_selected_row_in_narrow_space() {
    let panel = Panel::new(vec![process("running")]);
    let regular = visible(&panel, 80, 8);
    assert!(regular.contains("Ctrl+D stop"), "{regular}");
    assert_eq!(visible(&panel, 1, 1), "›");

    let stopping = visible(&Panel::new(vec![process("stopping")]), 80, 8);
    assert!(stopping.contains("running"), "{stopping}");
    assert!(!stopping.contains("stopping"), "{stopping}");
}
