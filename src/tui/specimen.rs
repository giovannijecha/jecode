// Design specimen painted by the real line renderer for review in a native
// terminal: cargo test --locked --offline specimen::specimen -- --ignored --nocapture
// Each component phase adds its own specimen beside this one, such as tools.
use super::{
    line::{Line, Span},
    text,
    theme::*,
};

const COLUMNS: usize = 120;

const FOREGROUNDS: &[(&str, &[(&str, Style)])] = &[
    (
        "Text",
        &[
            ("STRONG", STRONG),
            ("BODY", BODY),
            ("CODE_TEXT", CODE_TEXT),
            ("MUTED", MUTED),
            ("DIM", DIM),
            ("FAINT", FAINT),
        ],
    ),
    (
        "Interaction",
        &[
            ("ACCENT", ACCENT),
            ("LIT", LIT),
            ("KEYWORD", KEYWORD),
            ("BADGE_TEXT", BADGE_TEXT),
        ],
    ),
    (
        "Outcome",
        &[("GOOD", GOOD), ("BAD", BAD), ("WARNING", WARNING)],
    ),
    (
        "Shimmer",
        &[("S0", S0), ("S1", S1), ("S2", S2), ("S_BASE", S_BASE)],
    ),
];

const BACKGROUNDS: &[(&str, Surface)] = &[
    ("BASE", BASE),
    ("USER_SURFACE", USER_SURFACE),
    ("PILL_SURFACE", PILL_SURFACE),
    ("SELECTED_SURFACE", SELECTED_SURFACE),
    ("CODE_SURFACE", CODE_SURFACE),
    ("DIFF_ADDED_SURFACE", DIFF_ADDED_SURFACE),
    ("DIFF_REMOVED_SURFACE", DIFF_REMOVED_SURFACE),
    ("DIFF_ADDED_STRONG", DIFF_ADDED_STRONG),
    ("DIFF_REMOVED_STRONG", DIFF_REMOVED_STRONG),
    ("BADGE_READ", BADGE_READ),
    ("BADGE_COORD", BADGE_COORD),
    ("BADGE_EDIT", BADGE_EDIT),
    ("BADGE_SHELL", BADGE_SHELL),
    ("BADGE_IDLE", BADGE_IDLE),
    ("BADGE_BAD", BADGE_BAD),
    ("BADGE_WARN", BADGE_WARN),
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
    heading(rows, "Motion  ·  caret and shimmer, one frame per 100 ms");
    let mut line = Line::new("    ", BODY);
    for frame in [0, 4] {
        line = line.append(super::motion::caret(frame, None));
        line.push("  ", BODY);
    }
    rows.push(line);
    for frame in (0..15).step_by(2) {
        let mut line = Line::new("    ", BODY);
        line = line.append(super::motion::shimmer("Reading auth.rs", frame, false));
        rows.push(line);
    }
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

mod alignment;
mod chapter_live;
mod composer;
mod diffs;
mod discovery;
mod map;
mod menus;
mod messages;
mod notices;
mod panels;
mod permissions;
mod rhythm;
mod tinted;
mod tools;
mod work_chapters;
