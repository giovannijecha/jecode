// The `$` skill and `@` file sheets painted by the real renderer for review in a
// native terminal, wide and narrow, on dark and light inks:
// cargo test --locked --offline specimen::references -- --ignored --nocapture
use crate::{
    attachments::Attachment,
    skills::SkillInfo,
    test_support::Directory,
    tui::{
        editor::Editor,
        line::Line,
        references::{Browser, Origin},
        selector::Selector,
        state::State,
        theme::{self, MUTED, STRONG},
        view,
    },
};
use std::time::{Duration, Instant};

const WIDE: usize = 80;
const NARROW: usize = 44;
const MODEL: &str = "anthropic/claude-sonnet-5.5";
const DIRECTORY: &str = r"C:\Projects\example";

fn draft() -> Editor {
    let mut editor = Editor::default();
    editor.insert("Review ");
    editor.attach(Attachment {
        id: "att-specimen".into(),
        name: "shot.png".into(),
        media: "image/png".into(),
        size: 10,
        width: Some(1),
        height: Some(1),
        pages: None,
    });
    editor.insert(" against ");
    editor
}

fn origin(sigil: &str) -> (Editor, Origin) {
    let mut editor = draft();
    editor.insert(sigil);
    let end = editor.cursor;
    let origin = Origin {
        draft: editor.clone(),
        start: end - 1,
        end,
    };
    (editor, origin)
}

fn state(columns: usize, editor: Editor, selector: Option<Selector>) -> State {
    State {
        width: columns,
        height: 30,
        effort: "high".into(),
        editor,
        selector,
        ..State::default()
    }
}

fn skills(filter: &str, catalog: &[(&str, &str)], notes: Vec<String>, columns: usize) -> State {
    let (editor, origin) = origin("$");
    let catalog: Vec<_> = catalog
        .iter()
        .map(|(name, description)| SkillInfo {
            name: (*name).into(),
            description: (*description).into(),
        })
        .collect();
    let mut menu = Selector::skills(origin, &catalog, notes);
    menu.editor.insert(filter);
    menu.refresh();
    state(columns, editor, Some(menu))
}

const CATALOG: &[(&str, &str)] = &[
    (
        "code-review",
        "Review a branch or local changes for actionable defects",
    ),
    (
        "debugging",
        "Investigate an observed failure and prove a focused fix",
    ),
    (
        "feature-dev",
        "Implement a feature through to verified behavior",
    ),
    ("release", "Prepare a release without publishing it"),
];

fn workspace() -> Directory {
    let project = Directory::new();
    for path in [
        "src/main.rs",
        "src/tui/input.rs",
        "src/tui/editor.rs",
        "tools/main.rs",
        "docs/my notes.md",
        "docs/données.md",
        "target/debug/jecode.exe",
        "README.md",
    ] {
        let path = project.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "specimen").unwrap();
    }
    std::fs::write(project.path().join(".gitignore"), "target/\n").unwrap();
    std::fs::create_dir_all(project.path().join("empty")).unwrap();
    project
}

/// A browser at `directory` once its walk has delivered everything.
fn files(project: &Directory, directory: &str, filter: &str, columns: usize) -> State {
    let (editor, origin) = origin("@");
    let root = std::fs::canonicalize(project.path()).unwrap();
    let mut browser = Browser::new(origin, root, None);
    if !directory.is_empty() {
        browser.directory = directory.into();
        browser.load();
    }
    let started = Instant::now();
    while browser.pending() {
        browser.poll();
        assert!(started.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut menu = Selector::files(browser);
    menu.editor.insert(filter);
    menu.refresh();
    state(columns, editor, Some(menu))
}

fn inserted(columns: usize) -> State {
    let mut editor = draft();
    editor.insert(r#"$code-review @src/tui/input.rs and @"docs/my notes.md" "#);
    state(columns, editor, None)
}

fn states(project: &Directory, columns: usize) -> Vec<(&'static str, State)> {
    vec![
        ("Skills", skills("", CATALOG, vec![], columns)),
        ("Skills · filter", skills("rev", CATALOG, vec![], columns)),
        ("Skills · no match", skills("zzz", CATALOG, vec![], columns)),
        ("Skills · empty catalog", skills("", &[], vec![], columns)),
        (
            "Skills · skipped entries",
            skills(
                "",
                &[],
                vec!["2 skipped · /skills shows why".into()],
                columns,
            ),
        ),
        ("Files · workspace", files(project, "", "", columns)),
        ("Files · folder", files(project, "src", "", columns)),
        (
            "Files · filter, duplicate names",
            files(project, "", "main", columns),
        ),
        (
            "Files · typed path with spaces",
            files(project, "", r"docs\my no", columns),
        ),
        (
            "Files · explicit ignored path",
            files(project, "", "target/debug", columns),
        ),
        ("Files · empty folder", files(project, "empty", "", columns)),
        (
            "Files · unreadable folder",
            files(project, "removed", "", columns),
        ),
        ("Composer · references inserted", inserted(columns)),
    ]
}

#[test]
#[ignore = "prints the reference sheets specimen for review in a native terminal"]
fn references() {
    let project = workspace();
    let mut painted = String::new();
    for light in [false, true] {
        theme::set_light(light);
        for columns in [WIDE, NARROW] {
            let ink = if light { "light" } else { "dark" };
            painted.push_str(&format!(
                "\n{}\n\n",
                Line::new(
                    &format!("References · {columns} columns · {ink}"),
                    STRONG.bold()
                )
                .paint(WIDE)
            ));
            for (title, state) in states(&project, columns) {
                painted.push_str(&format!("{}\n", Line::new(title, MUTED).paint(columns)));
                for line in view::composer_frame(&state, MODEL, DIRECTORY).live {
                    painted.push_str(&format!("{}\n", line.paint(columns)));
                }
                painted.push('\n');
            }
        }
    }
    theme::set_light(false);
    print!("{painted}");
}
