// The command panel and every selector painted by the real renderer for review
// in a native terminal: cargo test --locked --offline specimen::menus -- --ignored --nocapture
use crate::{
    copy::Target,
    effort::Effort,
    openrouter::Model,
    sessions::Summary,
    tui::{
        line::Line,
        selector::Selector,
        state::State,
        theme::{MUTED, STRONG},
        view,
    },
};

const WIDE: usize = 80;
const NARROW: usize = 44;
const MODEL: &str = "anthropic/claude-sonnet-5.5";
const DIRECTORY: &str = r"C:\Projects\example";

fn state(columns: usize) -> State {
    State {
        width: columns,
        height: 30,
        effort: "high".into(),
        ..State::default()
    }
}

fn command(columns: usize, draft: &str) -> State {
    let mut state = state(columns);
    state.editor.insert(draft);
    state.suggestions.refresh(draft);
    state
}

fn selecting(columns: usize, selector: Selector) -> State {
    let mut state = state(columns);
    state.selector = Some(selector);
    state
}

fn model(id: &str, name: &str) -> Model {
    Model {
        id: id.into(),
        name: name.into(),
        prompt_price: None,
        completion_price: None,
        efforts: vec![Effort::Default, Effort::High, Effort::Medium, Effort::Low],
    }
}

fn models() -> Vec<Model> {
    [
        ("anthropic/claude-opus-5.5", "Anthropic: Claude Opus 5.5"),
        (
            "anthropic/claude-sonnet-5.5",
            "Anthropic: Claude Sonnet 5.5",
        ),
        ("anthropic/claude-haiku-5.5", "Anthropic: Claude Haiku 5.5"),
        ("openai/gpt-6", "OpenAI: GPT-6"),
        ("openai/gpt-6-mini", "OpenAI: GPT-6 Mini"),
        ("google/gemini-3.5-pro", "Google: Gemini 3.5 Pro"),
        ("google/gemini-3.5-flash", "Google: Gemini 3.5 Flash"),
        ("deepseek/deepseek-v4", "DeepSeek: V4"),
        ("mistralai/devstral-3", "Mistral: Devstral 3"),
        ("qwen/qwen4-coder", "Qwen: Qwen4 Coder"),
        ("x-ai/grok-5", "xAI: Grok 5"),
        ("moonshotai/kimi-k3", "MoonshotAI: Kimi K3"),
    ]
    .into_iter()
    .map(|(id, name)| model(id, name))
    .collect()
}

fn sessions() -> Vec<Summary> {
    let now = crate::sessions::now();
    [
        ("current", "Rate limiting for /login", 30),
        ("a", "Explain the session store", 4 * 60),
        ("b", "Fix the Windows console resize flicker", 3 * 3600),
        (
            "c",
            "Add persistent processes and local controls",
            2 * 86_400,
        ),
    ]
    .into_iter()
    .map(|(id, title, age)| Summary {
        id: id.into(),
        title: title.into(),
        updated: now - age * 1000,
        model: MODEL.into(),
    })
    .collect()
}

fn states(columns: usize) -> Vec<(&'static str, State)> {
    let mut drafts = state(columns);
    drafts.queue.push("also update the README").unwrap();
    drafts
        .queue
        .push("and bump the version\nin Cargo.toml")
        .unwrap();
    drafts.queue.pause();
    drafts.queue.push("run the full test suite").unwrap();
    let mut discard = Selector::drafts(&drafts.queue);
    drafts.selector = Some(Selector::drafts(&drafts.queue));
    let mut discarding = state(columns);
    discard.move_selection(false);
    discard.toggle_delete();
    discarding.selector = Some(discard);

    let mut search = Selector::models(&models(), false, MODEL);
    search.editor.insert("son");
    search.refresh();
    let mut delete = Selector::sessions(&sessions(), "current");
    delete.toggle_delete();
    let mut key = Selector::key();
    key.editor.insert("example-key-input");

    vec![
        ("Commands", command(columns, "/")),
        ("Commands · /co", command(columns, "/co")),
        (
            "Commands · arguments",
            command(columns, "/model openai/gpt-6"),
        ),
        ("Commands · no match", command(columns, "/xyz")),
        (
            "Model",
            selecting(columns, Selector::models(&models(), false, MODEL)),
        ),
        ("Model · search", selecting(columns, search)),
        (
            "Effort",
            selecting(
                columns,
                Selector::efforts(models().swap_remove(1), false, Effort::High),
            ),
        ),
        (
            "Settings",
            selecting(columns, Selector::settings(MODEL, Effort::High)),
        ),
        ("OpenRouter key", selecting(columns, key)),
        ("Loading", selecting(columns, Selector::loading())),
        (
            "Resume",
            selecting(columns, Selector::sessions(&sessions(), "current")),
        ),
        ("Resume · delete", selecting(columns, delete)),
        ("Drafts", drafts),
        ("Drafts · discard", discarding),
        (
            "Copy",
            selecting(
                columns,
                Selector::copy(&[
                    Target {
                        name: "Last response".into(),
                        text: "The limiter now returns 429 with a Retry-After header.".into(),
                    },
                    Target {
                        name: "Code block · rust".into(),
                        text: "const LOGIN_LIMIT: u32 = 10;".into(),
                    },
                ]),
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
#[ignore = "prints the menus specimen for review in a native terminal"]
fn menus() {
    let mut rows = Vec::new();
    for columns in [WIDE, NARROW] {
        heading(&mut rows, &format!("Menus  ·  {columns} columns"));
        for (title, state) in states(columns) {
            rows.push((Line::new(title, MUTED), columns));
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
