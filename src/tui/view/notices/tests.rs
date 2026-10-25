use super::*;
use std::time::Duration;

fn styled(line: &Line, text: &str) -> Style {
    line.spans
        .iter()
        .find(|span| span.text.contains(text))
        .unwrap_or_else(|| panic!("{text:?} not in {:?}", line.plain()))
        .style
}

#[test]
fn a_result_is_a_muted_line_without_a_marker_or_surface() {
    let rows = lines(
        &Feedback::result(Kind::Notice, "Complete · 48s · 6 tools"),
        80,
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].plain(), "Complete · 48s · 6 tools");
    assert_eq!(styled(&rows[0], "Complete"), MUTED);
    assert_eq!(styled(&rows[0], "48s"), MUTED);
    assert_eq!(rows[0].background, None);
}

#[test]
fn a_fact_is_muted_and_keeps_its_action_at_the_right_edge() {
    let notice =
        Feedback::result(Kind::Notice, "Previous session: Rate limiting").action("/resume", "open");
    let rows = lines(&notice, 60);
    assert_eq!(rows.len(), 1);
    assert_eq!(styled(&rows[0], "Previous session"), MUTED);
    assert!(
        rows[0]
            .plain()
            .starts_with("Previous session: Rate limiting")
    );
    assert!(rows[0].plain().ends_with("/resume  open"));
    assert_eq!(panel::cells(&rows[0]), 60);
}

#[test]
fn a_change_reads_old_to_new_with_the_new_value_strong() {
    let notice = Feedback::result(
        Kind::Notice,
        "Auto-compact: 80% → 55% · this and new conversations",
    )
    .change();
    let rows = lines(&notice, 80);
    assert_eq!(
        rows[0].plain(),
        "Auto-compact 80% → 55% · this and new conversations"
    );
    assert_eq!(styled(&rows[0], "Auto-compact"), MUTED);
    assert_eq!(styled(&rows[0], "80% →"), MUTED);
    assert_eq!(styled(&rows[0], "55%"), STRONG);
    assert_eq!(styled(&rows[0], "this and new"), MUTED);
}

#[test]
fn a_session_title_with_an_arrow_keeps_its_colon_and_muted_detail() {
    let rows = lines(
        &Feedback::result(Kind::Notice, "Previous session: Move A → B"),
        80,
    );
    assert_eq!(rows[0].plain(), "Previous session: Move A → B");
    assert_eq!(styled(&rows[0], "Move A → B"), MUTED);
}

#[test]
fn an_error_cause_after_a_colon_keeps_the_error_colour() {
    let rows = lines(
        &Feedback::result(
            Kind::Error,
            "Autosave failed: permission denied · prompt restored",
        ),
        80,
    );
    assert_eq!(
        rows[0].plain(),
        "▌ Autosave failed: permission denied · prompt restored"
    );
    assert_eq!(styled(&rows[0], "permission denied"), BAD);
    assert_eq!(styled(&rows[0], "prompt restored"), MUTED);
}

#[test]
fn warnings_and_errors_are_banners_one_cell_short_of_the_edge() {
    for (kind, stripe) in [(Kind::Warning, WARNING), (Kind::Error, BAD)] {
        let notice =
            Feedback::result(kind, "Interrupted · prompt restored").action("Alt+↑", "review");
        let rows = lines(&notice, 80);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].spans[0].text, "▌ ");
        assert_eq!(rows[0].spans[0].style, stripe);
        assert_eq!(styled(&rows[0], "Interrupted"), stripe);
        assert_eq!(rows[0].background, Some(USER_SURFACE));
        assert_eq!(panel::cells(&rows[0]), 79);
    }
}

#[test]
fn a_long_banner_wraps_under_its_subject_and_moves_the_action_to_its_own_row() {
    let notice = Feedback::result(
        Kind::Error,
        "OpenRouter HTTP 401 · the key was rejected · prompt restored",
    )
    .action("/settings", "update key");
    let rows = lines(&notice, 40);
    assert!(rows.len() >= 3);
    assert!(rows.iter().all(|row| row.background == Some(USER_SURFACE)));
    assert!(rows.iter().all(|row| panel::cells(row) <= 39));
    assert!(rows[1].plain().starts_with("  "));
    let last = rows.last().unwrap().plain();
    assert!(last.starts_with("  "));
    assert_eq!(last.trim(), "/settings  update key");
}

#[test]
fn progress_counts_down_to_the_next_attempt_and_retries_read_gold() {
    let retry = Feedback::progress(Kind::Warning, "Rate limited · HTTP 429 · attempt 2")
        .until(Instant::now() + Duration::from_millis(3200));
    let line = progress(&retry);
    assert_eq!(line.plain(), "Rate limited · HTTP 429 · attempt 2 in 4s");
    assert_eq!(styled(&line, "Rate limited"), WARNING);
    assert_eq!(styled(&line, "in 4s"), MUTED);
    let forms: Vec<_> = progress_forms(&retry).iter().map(Line::plain).collect();
    assert_eq!(
        forms,
        ["Rate limited · HTTP 429 in 4s", "Rate limited in 4s"]
    );

    let elapsed = Feedback::progress(Kind::Notice, "Compacting context")
        .until(Instant::now() - Duration::from_secs(1));
    let line = progress(&elapsed);
    assert_eq!(line.plain(), "Compacting context");
    assert_eq!(styled(&line, "Compacting"), STRONG);
}
