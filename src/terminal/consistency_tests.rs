use super::{
    Key, NAVIGATION_SEQUENCE, START_SEQUENCE, account, command_view,
    model::{Block, Model},
    render::Renderer,
    view,
    vt::Screen,
};
use crate::{
    command::Preview,
    session::{self, End, Event, Failure, Metrics},
};
use std::time::{Duration, Instant};

struct Surface {
    layout: view::Layout,
    renderer: Renderer,
    screen: Screen,
    size: (usize, usize),
}
impl Surface {
    fn new(size: (usize, usize)) -> Self {
        let mut screen = Screen::new(size.0, size.1);
        screen.feed("PS C:\\fixture> jecode\r\n");
        screen.feed(std::str::from_utf8(START_SEQUENCE).unwrap());
        Self {
            layout: view::Layout::default(),
            renderer: Renderer::default(),
            screen,
            size,
        }
    }
    fn draw(&mut self, model: &Model) -> String {
        let output = self.renderer.draw(
            self.layout.frame(model, self.size.0, self.size.1),
            self.size,
            false,
        );
        self.screen.feed(&output);
        output
    }
    fn chrome(&mut self, model: &Model) -> String {
        let frame = self
            .renderer
            .with_chrome(view::chrome(model, self.size.0, self.size.1));
        let output = self.renderer.draw(frame, self.size, false);
        self.screen.feed(&output);
        output
    }
    fn resize(&mut self, size: (usize, usize), model: &Model) {
        self.screen.resize(size.0, size.1);
        self.size = size;
        self.draw(model);
    }
    fn text(&self) -> String {
        self.screen.text()
    }
    fn clear_for_navigation(&mut self) {
        let frame = self.renderer.with_chrome(Vec::new());
        let output = self.renderer.draw(frame, self.size, false);
        self.screen.feed(&output);
        self.screen
            .feed(std::str::from_utf8(NAVIGATION_SEQUENCE).unwrap());
        self.screen
            .feed(std::str::from_utf8(START_SEQUENCE).unwrap());
        self.layout = view::Layout::default();
        self.renderer = Renderer::default();
    }
}

fn ready() -> (Model, session::Session) {
    let mut model = account::model(session::Model::Luna, None);
    account::event(&mut model, Event::Ready);
    (model, session::tests::ready_fixture())
}

fn rules(shown: &str) -> (usize, usize) {
    let indices: Vec<_> = shown
        .lines()
        .enumerate()
        .filter_map(|(index, line)| line.starts_with('─').then_some(index))
        .collect();
    assert_eq!(indices.len(), 2, "one composer and footer: {shown}");
    let lines: Vec<_> = shown.lines().collect();
    assert!(
        !lines[indices[1] + 1].is_empty(),
        "footer must follow the rule: {shown}"
    );
    (indices[0], indices[1])
}

fn active_row(shown: &str, label: &str) -> usize {
    let rows: Vec<_> = shown.lines().collect();
    let found: Vec<_> = rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| row.contains(label).then_some(index))
        .collect();
    assert_eq!(found.len(), 1, "one active label: {shown}");
    let index = found[0];
    assert!(matches!(
        rows[index].chars().next(),
        Some('\u{2800}'..='\u{28ff}')
    ));
    let (upper, _) = rules(shown);
    assert!(index < upper);
    assert_eq!(rows[upper - 1], "", "one gap before composer: {shown}");
    index
}

#[test]
fn startup_and_navigation_stay_near_the_shell_without_a_brand_or_extra_chrome() {
    let size = (80, 24);
    let mut surface = Surface::new(size);
    let mut model = account::model(session::Model::Luna, None);
    surface.draw(&model);
    let shown = surface.text();
    assert!(shown.contains("Connecting for sign-in / Esc cancels"));
    assert_eq!(shown.matches("jecode").count(), 1, "{shown}");
    assert_eq!(
        shown.lines().nth(1),
        Some("Connecting for sign-in / Esc cancels")
    );
    account::event(&mut model, Event::Ready);
    surface.draw(&model);
    let shown = surface.text();
    let lines: Vec<_> = shown.lines().collect();
    let (upper, _) = rules(&shown);
    assert_eq!(lines[0], "PS C:\\fixture> jecode");
    assert_eq!(upper, 1, "startup reserves no absent transcript: {shown}");
    assert_eq!(shown.matches("jecode").count(), 1, "{shown}");

    model.blocks.push(Block {
        speaker: "Assistant",
        text: "Previous answer.".into(),
    });
    surface.draw(&model);
    surface.clear_for_navigation();
    let (new_model, _) = ready();
    surface.draw(&new_model);
    let shown = surface.text();
    assert_eq!(shown.matches("Previous answer.").count(), 1, "{shown}");
    assert_eq!(shown.matches("jecode").count(), 1, "{shown}");
    rules(&shown);

    surface.clear_for_navigation();
    let (mut resumed, _) = ready();
    resumed.blocks.push(Block {
        speaker: "Assistant",
        text: "Restored answer.".into(),
    });
    surface.draw(&resumed);
    let shown = surface.text();
    assert_eq!(shown.matches("Previous answer.").count(), 1, "{shown}");
    assert_eq!(shown.matches("Restored answer.").count(), 1, "{shown}");
    rules(&shown);
}

#[test]
fn slash_menu_filters_and_closes_with_one_marker_and_adjacent_query() {
    let (mut model, mut session) = ready();
    let mut surface = Surface::new((80, 24));
    surface.draw(&model);
    account::input(&mut model, Key::Text("/".into()), &mut session);
    surface.draw(&model);
    let shown = surface.text();
    let rows: Vec<_> = shown.lines().collect();
    let (upper, lower) = rules(&shown);
    assert_eq!(rows[upper + 1], "› /new");
    assert_eq!(rows[upper + 7], "  /help");
    assert_eq!(rows[upper + 8], "  /login");
    assert_eq!(rows[upper + 9], "  /logout");
    assert_eq!(rows[upper + 10], "  / ");
    assert_eq!(lower, upper + 11, "menu and input must touch: {shown}");
    assert_eq!(rows[upper + 1..lower].join("\n").matches('›').count(), 1);

    account::input(&mut model, Key::Text("co".into()), &mut session);
    account::input(&mut model, Key::Down, &mut session);
    account::input(&mut model, Key::Left, &mut session);
    surface.draw(&model);
    let shown = surface.text();
    let rows: Vec<_> = shown.lines().collect();
    let (upper, lower) = rules(&shown);
    assert_eq!(rows[upper + 1], "  /context");
    assert_eq!(rows[upper + 2], "› /compact");
    assert_eq!(rows[upper + 3], "  /co");
    assert_eq!(lower, upper + 4);

    account::input(&mut model, Key::Escape, &mut session);
    surface.draw(&model);
    let shown = surface.text();
    let rows: Vec<_> = shown.lines().collect();
    let (upper, lower) = rules(&shown);
    assert_eq!(rows[upper + 1], "› /co");
    assert_eq!(lower, upper + 2);
    assert_eq!(shown.matches("/context").count(), 0, "{shown}");

    model.editor.take();
    let catalog = crate::providers::openai_account::catalog::Catalog::parse(br#"{"models":[{"slug":"gpt-5.6-luna","visibility":"list"},{"slug":"gpt-5.6-terra","visibility":"list"}]}"#).unwrap();
    model
        .menu
        .open(super::menu::models(&catalog, session::Model::Luna, false));
    surface.draw(&model);
    let shown = surface.text();
    let rows: Vec<_> = shown.lines().collect();
    let (upper, lower) = rules(&shown);
    assert_eq!(rows[upper + 1], "Model");
    assert!(rows[upper + 2].starts_with("› gpt-5.6-luna"));
    assert_eq!(rows[lower - 1], "   Filter…");
    assert_eq!(rows[upper + 1..lower].join("\n").matches('›').count(), 1);
}

#[test]
fn model_tool_command_approval_and_completion_replace_one_status_in_place() {
    let (mut model, mut session) = ready();
    let mut surface = Surface::new((80, 24));
    surface.draw(&model);
    account::input(
        &mut model,
        Key::Text("Inspect unique-fixture".into()),
        &mut session,
    );
    account::input(&mut model, Key::Enter, &mut session);
    surface.draw(&model);
    let shown = surface.text();
    let waiting = active_row(&shown, "Waiting for model");
    assert_eq!(shown.lines().nth(waiting - 1), Some(""));

    account::event(&mut model, Event::Thinking);
    surface.draw(&model);
    active_row(&surface.text(), "Thinking");
    let now = Instant::now();
    model.status_spinner.reset(now);
    assert!(!model.tick(now + Duration::from_millis(79)));
    assert!(model.tick(now + Duration::from_millis(80)));
    let output = surface.chrome(&model);
    assert!(
        !output.contains("unique-fixture"),
        "spinner replayed transcript"
    );
    assert_eq!(surface.text().matches("unique-fixture").count(), 1);

    account::input(&mut model, Key::Text("/".into()), &mut session);
    surface.draw(&model);
    let shown = surface.text();
    active_row(&shown, "Thinking");
    assert!(shown.contains("› /new\n"));
    assert!(shown.contains("  / \n"));
    account::input(&mut model, Key::Escape, &mut session);

    account::event(
        &mut model,
        Event::ToolStarted {
            name: "read_file",
            path: "src/main.rs".into(),
        },
    );
    surface.draw(&model);
    let shown = surface.text();
    let heading = active_row(&shown, "Exploring workspace");
    assert!(shown.lines().nth(heading + 1).unwrap().contains("Reading"));
    assert!(!shown.contains("Thinking"));
    account::event(
        &mut model,
        Event::ToolFinished {
            summary: "12 lines".into(),
            failed: false,
            limited: false,
        },
    );

    command_view::proposal(
        &mut model,
        7,
        Preview {
            command: "echo fixture".into(),
            cwd: ".".into(),
            shell: "fixture shell",
            timeout_seconds: 10,
        },
    );
    surface.draw(&model);
    let shown = surface.text();
    assert!(shown.contains("? Run this command"), "{shown}");
    assert!(shown.contains("Enter confirm"), "{shown}");
    assert!(!shown.contains("Exploring workspace"), "{shown}");
    assert!(!shown.contains("Waiting for model"), "{shown}");
    assert!(!model.tick(Instant::now() + Duration::from_secs(1)));
    rules(&shown);

    command_view::started(&mut model, 7);
    surface.draw(&model);
    active_row(&surface.text(), "Running command");
    command_view::finished(&mut model, 7, "Command finished".into(), true, false);
    account::event(&mut model, Event::RequestStarted);
    account::event(&mut model, Event::Text("A **checked** answer.".into()));
    surface.draw(&model);
    active_row(&surface.text(), "Streaming");
    account::event(
        &mut model,
        Event::Finished(End::Complete, Metrics::default()),
    );
    surface.draw(&model);
    let shown = surface.text();
    assert_eq!(shown.matches("unique-fixture").count(), 1, "{shown}");
    assert_eq!(shown.matches("Command finished").count(), 1, "{shown}");
    assert!(shown.contains("A checked answer."), "{shown}");
    assert!(shown.contains("Complete / 0.0s"), "{shown}");
    assert!(!shown.contains("⠋ Complete"), "{shown}");
    assert!(!shown.contains("Enter confirm"), "{shown}");
    assert!(!model.tick(Instant::now() + Duration::from_secs(2)));
    rules(&shown);
}

#[test]
fn long_and_short_responses_keep_one_transcript_and_footer_through_resize() {
    for answer in [
        "Short **Markdown** answer.\n\n```rust\nlet n = 2;\n```".to_owned(),
        (0..45).map(|n| format!("Paragraph-{n:02}\n\n")).collect(),
    ] {
        let (mut model, mut session) = ready();
        let mut surface = Surface::new((80, 24));
        account::input(
            &mut model,
            Key::Text("One unique question".into()),
            &mut session,
        );
        account::input(&mut model, Key::Enter, &mut session);
        surface.draw(&model);
        account::event(&mut model, Event::Text(answer));
        surface.draw(&model);
        surface.resize((55, 12), &model);
        account::event(
            &mut model,
            Event::Finished(End::Complete, Metrics::default()),
        );
        surface.draw(&model);
        surface.resize((90, 30), &model);
        let shown = surface.text();
        assert_eq!(shown.matches("One unique question").count(), 1, "{shown}");
        if shown.contains("Paragraph-00") {
            assert_eq!(shown.matches("Paragraph-00").count(), 1, "{shown}");
            assert_eq!(shown.matches("Paragraph-44").count(), 1, "{shown}");
        } else {
            assert!(shown.contains("Short Markdown answer."), "{shown}");
            assert!(shown.contains("let n = 2;"), "{shown}");
        }
        rules(&shown);
    }
}

#[test]
fn cancelled_and_failed_rows_stop_animation_and_reduced_motion_stays_static() {
    let (mut model, mut session) = ready();
    assert!(!model.tick(Instant::now() + Duration::from_secs(1)));
    model.tools.reduced_motion = true;
    let mut surface = Surface::new((80, 24));
    account::input(&mut model, Key::Text("Cancel fixture".into()), &mut session);
    account::input(&mut model, Key::Enter, &mut session);
    surface.draw(&model);
    assert!(surface.text().contains("⠿ Waiting for model"));
    let now = Instant::now();
    assert!(!model.tick(now + Duration::from_secs(2)));
    account::input(&mut model, Key::Escape, &mut session);
    surface.draw(&model);
    assert!(surface.text().contains("Stopping..."));
    account::event(
        &mut model,
        Event::Finished(End::Failed(Failure::Cancelled), Metrics::default()),
    );
    surface.draw(&model);
    let shown = surface.text();
    assert!(
        shown.contains("Interrupted / partial output retained"),
        "{shown}"
    );
    assert!(!shown.contains("Waiting for model"), "{shown}");
    assert!(!model.tick(now + Duration::from_secs(3)));
    rules(&shown);

    let (mut model, mut session) = ready();
    account::input(&mut model, Key::Text("Fail fixture".into()), &mut session);
    account::input(&mut model, Key::Enter, &mut session);
    account::event(
        &mut model,
        Event::Finished(End::Failed(Failure::Worker), Metrics::default()),
    );
    surface.clear_for_navigation();
    surface.draw(&model);
    let shown = surface.text();
    assert!(
        shown.contains("Account worker stopped unexpectedly"),
        "{shown}"
    );
    assert!(!shown.contains("Waiting for model"), "{shown}");
    assert!(!model.tick(now + Duration::from_secs(4)));
    rules(&shown);
}
