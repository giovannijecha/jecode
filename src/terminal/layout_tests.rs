use super::{
    block::BlockLayout,
    model::{Block, Model},
    render::Renderer,
    style::Tone,
    view::Layout,
};
use std::time::Instant;

#[test]
fn accented_user_panels_have_even_edges_and_keep_the_reserved_column() {
    let prompt = "Esamina questa cartella e spiegami come è organizzato il progetto, citando i file che hai letto.";
    for source in [
        prompt,
        "Caffè già pronto: più qualità. À bientôt!",
        "Łódź Œuvre",
    ] {
        for columns in [25, 60, 120] {
            let mut block = BlockLayout::new(columns - 1);
            let rows = block.rows(&Block {
                speaker: "You",
                text: source.into(),
            });
            assert!(rows.len() >= 3);
            for row in rows {
                assert_eq!(row.tone, Tone::User);
                // These ASCII/Latin letters each occupy one display cell. Do not
                // use production width() as the oracle for the padding defect.
                assert_eq!(row.text.chars().count(), columns - 1, "{row:?}");
                assert_eq!(row.paint(false), row.text);
                assert!(!row.paint(false).contains('\x1b'));
            }
        }
    }
}

#[test]
fn empty_model_requests_do_not_add_gaps_or_repaint_the_transcript() {
    let mut model = Model::new(Instant::now());
    model.blocks = vec![Block {
        speaker: "You",
        text: "Inspect this workspace.".into(),
    }];
    let mut layout = Layout::default();
    let mut renderer = Renderer::default();
    let size = (120, 36);
    let before = layout.frame(&model, size.0, size.1);
    renderer.draw(before.clone(), size, false);
    for path in [".", "src", "docs"] {
        model.blocks.push(Block {
            speaker: "Assistant",
            text: String::new(),
        });
        let frame = layout.frame(&model, size.0, size.1);
        assert!(renderer.draw(frame, size, false).is_empty());
        model.blocks.push(Block {
            speaker: "Tool",
            text: format!("list_files / {path}\n  3 entries / 0 omitted"),
        });
        renderer.draw(layout.frame(&model, size.0, size.1), size, false);
    }
    let with_empty = layout.frame(&model, size.0, size.1);
    model.blocks.retain(|block| !block.text.is_empty());
    assert_eq!(with_empty, Layout::default().frame(&model, size.0, size.1));
    let rows: Vec<_> = with_empty.iter().map(|row| row.text.as_str()).collect();
    let first = rows
        .iter()
        .position(|row| *row == "list_files / .")
        .unwrap();
    assert_eq!(
        &rows[first..first + 9],
        [
            "list_files / .",
            "  3 entries / 0 omitted",
            "",
            "list_files / src",
            "  3 entries / 0 omitted",
            "",
            "list_files / docs",
            "  3 entries / 0 omitted",
            "",
        ]
    );
}

#[test]
fn transcript_boundaries_use_one_gap_without_changing_code_or_user_padding() {
    let now = Instant::now();
    let mut model = Model::new(now);
    model.blocks = vec![
        Block {
            speaker: "You",
            text: "Inspect this.".into(),
        },
        Block {
            speaker: "Assistant",
            text: "\n\nAnswer.\n\n\n".into(),
        },
        Block {
            speaker: "Assistant",
            text: "\n  \n".into(),
        },
        Block {
            speaker: "ToolSummary",
            text: "✓ Explored workspace".into(),
        },
    ];
    let rows = Layout::default().frame(&model, 80, 24);
    let answer = rows.iter().position(|r| r.text == "Answer.").unwrap();
    let tool = rows
        .iter()
        .position(|r| r.text == "✓ Explored workspace")
        .unwrap();
    assert_eq!(tool - answer, 2, "one neutral gap between visible blocks");
    assert_eq!(rows[answer - 1].tone, Tone::Text);
    assert!(rows[answer - 1].text.is_empty());
    assert_eq!(rows[answer - 2].tone, Tone::User);
    assert!(
        rows[answer - 2].text.trim().is_empty(),
        "keep panel padding"
    );
    let before = rows;
    model.blocks.push(Block {
        speaker: "Assistant",
        text: "\n\n".into(),
    });
    model.blocks.push(Block {
        speaker: "Status",
        text: "\n  \n".into(),
    });
    assert_eq!(before, Layout::default().frame(&model, 80, 24));

    let mut code = BlockLayout::new(79);
    let rows = code.rows(&Block {
        speaker: "Assistant",
        text: "```\n\nkept\n\n```".into(),
    });
    assert_eq!(
        rows.iter()
            .filter(|r| r.tone == Tone::Code && r.text.trim().is_empty())
            .count(),
        2
    );
}

#[test]
fn action_panels_separate_metadata_body_and_outcome_by_one_row() {
    for (speaker, source, body, footer) in [
        (
            "CommandPreview",
            "  Run command\n  $ test\n  cwd: demo\n  Approved once\n  output one\n  output two\n✓ Complete",
            "  output one",
            "✓ Complete",
        ),
        (
            "CommandPreview",
            "  Run command\n  $ test\n  cwd: demo\n  Approved once\n  output one\n  output two\n! Failed",
            "  output one",
            "! Failed",
        ),
        (
            "EditPreview",
            "  Edit example.rs\n  @@ example\n- old\n+ new\n  Approved once\n✓ Complete",
            "- old",
            "  Approved once",
        ),
    ] {
        let mut layout = BlockLayout::new(79);
        let rows = layout.rows(&Block {
            speaker,
            text: source.into(),
        });
        let body = rows.iter().position(|r| r.text.starts_with(body)).unwrap();
        let footer = rows
            .iter()
            .position(|r| r.text.starts_with(footer))
            .unwrap();
        for gap in [body - 1, footer - 1] {
            assert!(rows[gap].text.is_empty(), "missing gap: {rows:?}");
            assert_eq!(rows[gap].tone, Tone::Text);
            assert!(!rows[gap - 1].text.trim().is_empty());
        }
        assert_eq!(footer - body, 3, "body rows must stay compact");
        assert!(!rows.first().unwrap().text.is_empty());
        assert!(!rows.last().unwrap().text.is_empty());
    }
}

#[test]
fn command_denied_before_output_has_one_gap_before_its_outcome() {
    let mut layout = BlockLayout::new(79);
    let rows = layout.rows(&Block {
        speaker: "CommandPreview",
        text: "  Run command\n  $ test\n  cwd: demo\n· Denied".into(),
    });
    assert_eq!(rows[3].text, "");
    assert_eq!(rows[4].text, "· Denied");
}

#[test]
fn runtime_activity_is_above_the_composer_and_approval_stays_inside_at_small_sizes() {
    let now = Instant::now();
    for prompt in ["/edit", "/command", "/tools"] {
        let mut model = Model::new(now);
        model.input(super::Key::Text(prompt.into()), now);
        model.input(super::Key::Enter, now);
        model.tick(now);
        for size in [(80, 24), (25, 9)] {
            let rows = super::view::chrome(&model, size.0, size.1);
            assert!(rows.len() < size.1);
            let rule = rows.iter().position(|r| r.text.starts_with('─')).unwrap();
            let bottom = rows.iter().rposition(|r| r.text.starts_with('─')).unwrap();
            assert_eq!(rows.len() - bottom - 1, 1, "{rows:?}");
            if prompt == "/tools" {
                assert!(rule > 0, "activity missing above input: {rows:?}");
                assert!(
                    rows[..rule]
                        .iter()
                        .any(|r| r.text.contains("Exploring workspace"))
                );
                assert!(
                    rows[rule + 1..bottom]
                        .iter()
                        .all(|r| !r.text.contains("Exploring workspace"))
                );
                assert!(
                    rows[rule + 1..bottom]
                        .iter()
                        .any(|r| r.text.contains("Ask anything"))
                );
            } else {
                assert_eq!(
                    rule, 0,
                    "pending approval has no running activity: {rows:?}"
                );
                assert!(
                    rows[rule + 1..bottom]
                        .iter()
                        .any(|r| r.text.contains("Enter confirm"))
                );
            }
        }
    }
}

#[test]
fn streaming_action_spacing_only_appends_content_before_the_boundary_gap() {
    let now = Instant::now();
    let mut model = Model::new(now);
    model.input(super::Key::Text("/command".into()), now);
    model.input(super::Key::Enter, now);
    let mut layout = Layout::default();
    let committed = |rows: Vec<super::style::Row>| {
        let mut rows: Vec<_> = rows.into_iter().take_while(|r| !r.transient).collect();
        assert!(rows.last().unwrap().text.is_empty());
        rows.pop(); // The single block boundary can become an internal separator.
        rows
    };
    let mut previous = committed(layout.frame(&model, 100, 24));
    model.input(super::Key::Right, now);
    model.input(super::Key::Enter, now);
    for step in 0..=4 {
        model.tick(now + std::time::Duration::from_millis(650 * step));
        let next = committed(layout.frame(&model, 100, 24));
        assert!(
            next.starts_with(&previous),
            "existing output moved during streaming"
        );
        previous = next;
    }
}
