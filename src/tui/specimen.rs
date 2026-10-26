// Design specimen painted by the real line renderer for review in a native
// terminal: cargo test --locked --offline specimen::specimen -- --ignored --nocapture
// Each component phase adds its own specimen beside this one, such as tools.
use super::{
    activity,
    line::{Line, Span},
    text,
    theme::*,
};

const COLUMNS: usize = 120;

const FOREGROUNDS: &[(&str, &[(&str, Style)])] = &[
    (
        "Text",
        &[
            ("EMPHASIS", EMPHASIS),
            ("STRONG", STRONG),
            ("CODE_TEXT", CODE_TEXT),
            ("MUTED", MUTED),
            ("TOOL_DIM", TOOL_DIM),
        ],
    ),
    (
        "Interaction",
        &[
            ("ACCENT", ACCENT),
            ("TOOL_ACCENT", TOOL_ACCENT),
            ("KEYWORD", KEYWORD),
        ],
    ),
    (
        "Outcome",
        &[("GOOD", GOOD), ("BAD", BAD), ("WARNING", WARNING)],
    ),
];

const BACKGROUNDS: &[(&str, Surface)] = &[
    ("USER_SURFACE", USER_SURFACE),
    ("SELECTED_SURFACE", SELECTED_SURFACE),
    ("CODE_SURFACE", CODE_SURFACE),
    ("DIFF_ADDED_SURFACE", DIFF_ADDED_SURFACE),
    ("DIFF_REMOVED_SURFACE", DIFF_REMOVED_SURFACE),
    ("DIFF_ADDED_STRONG", DIFF_ADDED_STRONG),
    ("DIFF_REMOVED_STRONG", DIFF_REMOVED_STRONG),
];

const ATTRIBUTES: &[(&str, Style)] = &[
    ("bold", STRONG.bold()),
    ("italic", BODY.italic()),
    ("underline", BODY.underline()),
    ("strike", BODY.strike()),
    ("bold italic", STRONG.bold().italic()),
    ("link", LINK),
];

const GLYPHS: &[(&str, &[(&str, &str)])] = &[
    (
        "Markers",
        &[
            ("●", "conversation bullet"),
            ("⏺", "macOS bullet"),
            ("▰", "scanner lit"),
            ("▱", "scanner resting"),
            ("❯", "prompt and focus"),
            ("›", "current prompt"),
            ("▸", "collapsed"),
            ("▾", "expanded"),
            ("▶", "background"),
        ],
    ),
    (
        "Structure",
        &[
            ("⎿", "result connector"),
            ("└", "last branch"),
            ("├", "branch"),
            ("│", "guide"),
            ("─", "rule"),
            ("━", "context meter"),
            ("╭", "round corner"),
            ("▎", "quote bar"),
            ("┃", "heavy bar"),
        ],
    ),
    (
        "Outcome",
        &[
            ("✓", "success"),
            ("✔", "heavy success"),
            ("✗", "failure"),
            ("×", "current failure"),
            ("■", "stopped"),
            ("▲", "warning"),
            ("⚠", "emoji-prone warning"),
            ("○", "idle"),
            ("◉", "selected radio"),
        ],
    ),
    (
        "Activity",
        &[
            ("·", "frame"),
            ("✢", "frame"),
            ("✳", "frame"),
            ("✶", "frame"),
            ("✻", "frame"),
            ("✽", "frame"),
            ("⠋", "braille frame"),
            ("◐", "half circle"),
            ("∴", "pulse"),
        ],
    ),
    (
        "Keys",
        &[
            ("↑", "up"),
            ("↓", "down"),
            ("↵", "enter"),
            ("⏎", "enter alternative"),
            ("⇥", "tab"),
            ("…", "ellipsis"),
            ("☐", "open task"),
            ("☑", "done task"),
            ("☒", "alternate done task"),
        ],
    ),
];

fn rgb(style: impl std::fmt::Display) -> String {
    let style = style.to_string();
    let fields: Vec<&str> = style.split(';').collect();
    fields
        .windows(5)
        .find(|window| matches!(window[0], "38" | "48") && window[1] == "2")
        .map(|window| {
            let channel = |value: &str| value.parse::<u8>().unwrap_or(0);
            format!(
                "#{:02X}{:02X}{:02X}",
                channel(window[2]),
                channel(window[3]),
                channel(window[4])
            )
        })
        .unwrap_or_else(|| "inherit".into())
}

fn on(line: &mut Line, value: &str, style: Style, background: Surface) {
    line.spans.push(Span {
        text: value.into(),
        style,
        background: Some(background),
    });
}

fn heading(rows: &mut Vec<Line>, title: &str) {
    rows.push(Line::default());
    rows.push(Line::new(title, STRONG.bold()));
}

fn palette(rows: &mut Vec<Line>) {
    heading(
        rows,
        "Foregrounds  ·  terminal, user, selected and code surfaces",
    );
    for (group, roles) in FOREGROUNDS {
        rows.push(Line::new(&format!("  {group}"), MUTED));
        for (name, style) in *roles {
            let mut line = Line::new(&format!("    {name:<14}{:<10}", rgb(*style)), MUTED);
            line.push("███  Sample text  ", *style);
            for background in [USER_SURFACE, SELECTED_SURFACE] {
                on(&mut line, " Sample text ", *style, background);
                line.push("  ", BODY);
            }
            rows.push(line);
        }
    }
    heading(rows, "Surfaces  ·  main text roles on each background");
    for (name, background) in BACKGROUNDS {
        let mut line = Line::new(&format!("    {name:<24}{:<10}", rgb(*background)), MUTED);
        on(&mut line, " ", BODY, *background);
        for (sample, style) in [
            ("Text", STRONG),
            ("Muted", MUTED),
            ("Accent", ACCENT),
            ("Good", GOOD),
            ("Bad", BAD),
            ("Warning", WARNING),
        ] {
            on(&mut line, &format!("{sample} "), style, *background);
        }
        rows.push(line);
    }
    let mut caret = Line::new(&format!("    {:<24}{:<10}", "CURSOR", rgb(CURSOR)), MUTED);
    caret.push("A", CURSOR);
    caret.push("sk anything…", MUTED);
    rows.push(caret);
}

fn attributes(rows: &mut Vec<Line>) {
    heading(rows, "Attributes");
    let mut line = Line::new("    ", BODY);
    for (name, style) in ATTRIBUTES {
        line.push(name, *style);
        line.push("   ", BODY);
    }
    rows.push(line);
}

fn scanner(rows: &mut Vec<Line>) {
    heading(rows, "Activity scanner  ·  8 frames at 120 ms");
    let mut line = Line::new("    ", BODY);
    for frame in 0..8 {
        for span in activity::indicator(frame).spans {
            line.push(&span.text, span.style);
        }
        line.push("  ", BODY);
    }
    rows.push(line);
}

fn glyphs(rows: &mut Vec<Line>) {
    heading(
        rows,
        "Glyphs  ·  every bar must line up; a shifted bar means the terminal draws two cells",
    );
    rows.push(Line::new("    x|  reference cell", MUTED));
    for (group, entries) in GLYPHS {
        rows.push(Line::new(&format!("  {group}"), MUTED));
        for (glyph, role) in *entries {
            let character = glyph.chars().next().unwrap_or(' ');
            let mut line = Line::new("    ", BODY);
            line.push(glyph, STRONG);
            line.push("|", ACCENT);
            line.push(
                &format!(
                    "  U+{:04X}  jecode width {}  {role}",
                    character as u32,
                    text::width(character)
                ),
                MUTED,
            );
            rows.push(line);
        }
    }
}

#[test]
#[ignore = "prints the design specimen for review in a native terminal"]
fn specimen() {
    let mut rows = vec![Line::new("Jecode design specimen", STRONG.bold())];
    palette(&mut rows);
    attributes(&mut rows);
    scanner(&mut rows);
    glyphs(&mut rows);
    // One write keeps the output whole when the tool specimen runs alongside.
    let painted: String = rows
        .iter()
        .map(|row| format!("{}\n", row.paint(COLUMNS)))
        .collect();
    print!("{painted}");
}

mod chapter_live;
mod composer;
mod menus;
mod messages;
mod notices;
mod panels;
mod rhythm;
mod tinted;
mod tools;
mod work_chapters;
