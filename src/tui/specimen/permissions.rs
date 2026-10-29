// Permission requests, menus and the level indicator painted by the real
// composer renderer for review in a native terminal:
// cargo test --locked --offline specimen::permissions -- --ignored --nocapture
use crate::permissions::{Access, Grant, Kind, Level, Request};
use crate::tui::{
    line::Line,
    permission::Pending,
    selector::Selector,
    state::State,
    theme::{MUTED, STRONG},
    view,
};

const WIDE: usize = 80;
const NARROW: usize = 44;
const HEIGHT: usize = 30;
const MODEL: &str = "anthropic/claude-sonnet-5.5";
const DIRECTORY: &str = r"C:\Projects\example";
const FILE: &str = r"C:\Projects\webterminal\src\session.rs";

fn state(columns: usize) -> State {
    State {
        width: columns,
        height: HEIGHT,
        effort: "high".into(),
        ..State::default()
    }
}

fn access(kind: Kind, target: &str, place: Option<&str>) -> Access {
    Access {
        kind,
        target: target.into(),
        place: place.map(Into::into),
        outside: true,
        base: None,
        identity: None,
    }
}

fn asking(columns: usize, tool: &str, access: Access, agent: &str, waiting: usize) -> State {
    let mut state = state(columns);
    state.editor.insert("kept draft");
    state.permission = Some(Pending {
        request: Request {
            id: 1,
            call_id: "call-1".into(),
            agent: None,
            tool: tool.into(),
            access,
        },
        waiting,
        agent: agent.into(),
        focus: None,
    });
    state
}

fn selecting(columns: usize, level: Level, selector: Selector) -> State {
    let mut state = state(columns);
    state.permission_level = level;
    state.selector = Some(selector);
    state
}

fn grants() -> Vec<Grant> {
    vec![
        access(Kind::Reads, FILE, None).grant(),
        access(Kind::Edits, FILE, None).grant(),
        access(
            Kind::Runs,
            "cargo test session",
            Some(r"C:\Projects\webterminal"),
        )
        .grant(),
    ]
}

fn states(columns: usize) -> Vec<(&'static str, State)> {
    let read = access(Kind::Reads, FILE, None);
    let edit = access(Kind::Edits, FILE, None);
    let run = access(
        Kind::Runs,
        "cargo test session",
        Some(r"C:\Projects\webterminal"),
    );
    let mut focused = asking(columns, "edit", edit, "MAIN AGENT", 1);
    focused.permission.as_mut().unwrap().focus = Some(1);
    let mut composer = state(columns);
    composer.permission_level = Level::Read;
    vec![
        ("Level indicator · Read", composer),
        (
            "Request · main agent read",
            asking(columns, "read", read, "MAIN AGENT", 1),
        ),
        ("Request · edit, focus on remember", focused),
        (
            "Request · collaborator command, 1 of 3",
            asking(columns, "exec", run, "AGENT 1", 3),
        ),
        (
            "/permissions",
            selecting(
                columns,
                Level::Workspace,
                Selector::permissions(Level::Workspace, Level::Workspace, 3),
            ),
        ),
        (
            "Remembered · paused at Read",
            selecting(
                columns,
                Level::Read,
                Selector::remembered(grants(), Level::Read),
            ),
        ),
        (
            "Settings › Default permissions",
            selecting(
                columns,
                Level::Workspace,
                Selector::default_permissions(Level::Workspace),
            ),
        ),
    ]
}

fn heading(rows: &mut Vec<(Line, usize)>, title: &str) {
    rows.push((Line::default(), WIDE));
    rows.push((Line::new(title, STRONG.bold()), WIDE));
    rows.push((Line::default(), WIDE));
}

#[test]
#[ignore = "prints the permissions specimen for review in a native terminal"]
fn permissions() {
    let mut rows = Vec::new();
    for columns in [WIDE, NARROW] {
        heading(&mut rows, &format!("Permissions  ·  {columns} columns"));
        for (title, state) in states(columns) {
            rows.push((Line::new(title, MUTED), columns));
            rows.push((Line::default(), columns));
            for line in view::composer_frame(&state, MODEL, DIRECTORY).live {
                rows.push((line, columns));
            }
            rows.push((Line::default(), columns));
        }
    }
    let painted: String = rows
        .iter()
        .map(|(line, columns)| format!("{}\n", line.paint(*columns)))
        .collect();
    print!("{painted}");
}
