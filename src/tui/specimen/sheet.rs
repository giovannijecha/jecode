// The sheet and the commands key, painted by the real renderer on both palettes at the
// narrow, usual and wide widths, for review in a native terminal:
// cargo test --locked --offline specimen::sheet -- --ignored --nocapture
use crate::tui::{
    commands::busy_waits,
    line::Line,
    selector::Selector,
    state::State,
    theme::{self, MUTED, STRONG},
    view,
};

const MODEL: &str = "anthropic/claude-sonnet-5.5";
const DRAFT: &str = "Also check the token refresh on wake";

fn state(columns: usize) -> State {
    let mut state = State {
        width: columns,
        height: 30,
        effort: "high".into(),
        context: Some(crate::context::Status {
            tokens: Some(122_400),
            capacity: Some(200_000),
            auto_compact: 80,
            pending: None,
        }),
        ..State::default()
    };
    state.editor.insert(DRAFT);
    state
}

fn scenes(columns: usize) -> Vec<(&'static str, State)> {
    let sheet = |selector: Selector| {
        let mut state = state(columns);
        state.selector = Some(selector);
        state
    };
    let mut filtered = Selector::commands(|_| false);
    filtered.editor.insert("mod");
    filtered.refresh();
    vec![
        ("Composer with a draft", state(columns)),
        ("Commands", sheet(Selector::commands(|_| false))),
        ("Commands, filtered", sheet(filtered)),
        (
            "Commands during a turn",
            sheet(Selector::commands(busy_waits)),
        ),
        (
            "Settings",
            sheet(Selector::settings(
                MODEL,
                crate::effort::Effort::High,
                80,
                Default::default(),
                &crate::backend::openrouter::Routing::Auto,
                Default::default(),
                Default::default(),
            )),
        ),
        ("Theme", sheet(Selector::theme(Default::default()))),
    ]
}

#[test]
#[ignore = "prints the sheet on both palettes for review in a native terminal"]
fn sheet() {
    for (palette, light) in [("Dark inks", false), ("Light inks", true)] {
        theme::set_light(light);
        for columns in [44, 80, 120] {
            println!(
                "{}",
                Line::new(&format!("{palette} · {columns} columns"), STRONG).paint(columns)
            );
            for (title, state) in scenes(columns) {
                println!("{}", Line::default().paint(columns));
                println!("{}", Line::new(title, MUTED).paint(columns));
                let frame = view::composer_frame(&state, MODEL, r"C:\Projects\example");
                for line in frame.live {
                    println!("{}", line.paint(columns));
                }
            }
            println!("{}", Line::default().paint(columns));
        }
    }
    theme::set_light(false);
}
