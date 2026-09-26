use super::*;
use crate::terminal::lab::glyph::UNICODE;

fn tool(verb: &str, subject: &str, status: Status, detail: Detail) -> Tool {
    Tool {
        verb: verb.into(),
        subject: subject.into(),
        summary: String::new(),
        status,
        elapsed_ms: 1_400,
        detail,
    }
}

fn texts(rows: &[Row]) -> Vec<&str> {
    rows.iter().map(|row| row.text.as_str()).collect()
}

#[test]
fn steps_form_a_tree_with_detail_under_the_verb() {
    let mut read = tool(
        "Read",
        "src/main.rs",
        Status::Done,
        Detail::Output(String::new()),
    );
    read.summary = "124 lines".into();
    let run = tool(
        "Run",
        "cargo test",
        Status::Running,
        Detail::Output("ok".into()),
    );
    let rows = rows(&[read, run], 42, &UNICODE, "⠲", false);
    assert_eq!(
        texts(&rows),
        [
            " ├─ ✓ Read src/main.rs · 124 lines   1.4s",
            " └─ ⠲ Run cargo test                 1.4s",
            "      ok",
        ]
    );
    assert!(rows.iter().all(|row| text::width(&row.text) <= 41));
}

#[test]
fn inner_detail_keeps_the_stem_and_long_output_keeps_its_tail() {
    let output = (1..=8)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let run = tool("Run", "make", Status::Failed, Detail::Output(output));
    let done = tool("Read", "a", Status::Done, Detail::Output(String::new()));
    let rows = rows(&[run, done], 30, &UNICODE, "", false);
    assert_eq!(rows[1].text, " │    … 3 earlier lines");
    assert_eq!(rows[2].text, " │    line 4");
    assert_eq!(rows[6].text, " │    line 8");
    assert_eq!(rows[6].spans.last().unwrap().1, Tone::Error);
    assert!(rows[7].text.starts_with(" └─ ✓ Read a"));
}

#[test]
fn diffs_keep_their_head_as_colored_bands() {
    let diff = "@@ -1 +1 @@\n-old\n+new\n ctx\n+1\n+2\n+3\n+4\n+5\n+6";
    let edit = tool("Edit", "lib.rs", Status::Done, Detail::Diff(diff.into()));
    let rows = rows(&[edit], 24, &UNICODE, "", false);
    assert_eq!(rows.len(), 1 + 8 + 1);
    assert_eq!(rows[2].text, "      -old             ");
    assert_eq!(rows[2].spans.last().unwrap().1, Tone::Removed);
    assert_eq!(rows[3].spans.last().unwrap().1, Tone::Added);
    assert_eq!(rows[9].text, "      … 2 more lines");
    let wide = super::rows(
        &[tool("Edit", "a", Status::Done, Detail::Diff(diff.into()))],
        40,
        &UNICODE,
        "",
        false,
    );
    assert_eq!(wide[9].text, "      … 2 more lines · ctrl+o");
}

#[test]
fn narrow_rows_clip_the_subject_and_drop_the_summary() {
    let mut read = tool(
        "Read",
        "src/terminal/render.rs",
        Status::Done,
        Detail::Output(String::new()),
    );
    read.summary = "124 lines".into();
    let rows = rows(&[read], 28, &UNICODE, "", false);
    assert_eq!(rows[0].text, " └─ ✓ Read src/termi…  1.4s");
}

#[test]
fn expanded_detail_shows_every_line_without_a_marker() {
    let lines = |prefix: &str, count: usize| {
        (1..=count)
            .map(|n| format!("{prefix}{n}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let run = tool(
        "Run",
        "make",
        Status::Done,
        Detail::Output(lines("line ", 8)),
    );
    let edit = tool("Edit", "lib.rs", Status::Done, Detail::Diff(lines("+", 12)));
    let rows = rows(&[run, edit], 30, &UNICODE, "", true);
    assert_eq!(rows.len(), 1 + 8 + 1 + 12);
    assert_eq!(rows[1].text, " │    line 1");
    assert!(rows.iter().all(|row| !row.text.contains("ctrl+o")));
}
