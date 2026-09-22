//! Exercise owned Windows input and the menu renderer without network or user state.
use super::*;
#[allow(dead_code)]
#[path = "../../tests/support/conpty.rs"]
mod conpty;
use std::{fs, path::PathBuf, time::Duration};

#[test]
fn native_console_child() {
    let Some(directory) = std::env::var_os("JECODE_TUI_TEST_DIR") else {
        return;
    };
    let _handles = conpty::bind_test_io();
    let directory = PathBuf::from(directory);
    let mut terminal = platform::Terminal::open().unwrap();
    let mut session = crate::session::tests::ready_fixture();
    let mut model = account::model(crate::session::Model::Luna, None);
    account::event(&mut model, crate::session::Event::Ready);
    model.blocks.push(model::Block {
        speaker: "Assistant",
        text: "Retained answer in scrollback.".into(),
    });
    let mut renderer = render::Renderer::default();
    let mut layout = view::Layout::default();
    let mut previous_request = String::new();
    let mut output = io::stdout().lock();
    loop {
        let size = terminal.size().unwrap();
        output
            .write_all(
                renderer
                    .draw(layout.frame(&model, size.0, size.1), size, true)
                    .as_bytes(),
            )
            .unwrap();
        output.flush().unwrap();
        if let Ok(request) = fs::read_to_string(directory.join("request"))
            && request != previous_request
        {
            previous_request = request;
            fs::write(
                directory.join(format!("snapshot-{previous_request}")),
                conpty::snapshot(),
            )
            .unwrap();
        }
        for key in terminal.poll().unwrap() {
            account::input(&mut model, key, &mut session);
        }
        while let Some(event) = session.poll() {
            account::event(&mut model, event);
        }
        if model.quit {
            return;
        }
    }
}

fn wait_for(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "native menu did not settle");
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn snapshot(path: &std::path::Path, name: &str) -> String {
    fs::write(path.join("request"), name).unwrap();
    let file = path.join(format!("snapshot-{name}"));
    wait_for(|| fs::read_to_string(&file).is_ok_and(|s| !s.is_empty()));
    fs::read_to_string(file).unwrap()
}

fn inside_composer(screen: &str, text: &str) {
    let rows: Vec<_> = screen.lines().collect();
    let rules: Vec<_> = rows
        .iter()
        .enumerate()
        .filter(|(_, line)| line.starts_with('─'))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(rules.len(), 2, "{screen}");
    let at = rows.iter().position(|line| line.contains(text)).unwrap();
    assert!(rules[0] < at && at < rules[1], "{screen}");
}

#[test]
fn native_menu_selects_with_arrows_and_preserves_transcript_across_resize() {
    let fixture = crate::state::tests::Fixture::new();
    let mut console = conpty::Console::start_named(
        &fixture.0,
        "terminal::menu_native_tests::native_console_child",
    );
    wait_for(|| console.output().contains("Retained answer"));
    console.input.write_all(b"/").unwrap();
    wait_for(|| console.output().contains("/resume"));
    let wide = snapshot(&fixture.0, "wide");
    println!("{wide}");
    assert!(wide.contains("/resume") && wide.contains("/model"));
    inside_composer(&wide, "/resume");
    assert!(!wide.contains("Commands") && !wide.contains("choose"));
    assert!(!wide.contains("Ctrl+Q"));
    assert_eq!(wide.matches("Retained answer").count(), 1);
    // New -> resume -> model. These are actual native console key events.
    console.input.write_all(b"\x1b[B\x1b[B\r").unwrap();
    wait_for(|| console.output().contains("· current"));
    console.resize(48, 24);
    console.input.write_all(b"\x1b[B\r").unwrap();
    wait_for(|| console.output().contains("gpt-5.6-terra · medium"));
    let selected = snapshot(&fixture.0, "selected");
    assert!(selected.contains("gpt-5.6-terra") && !selected.contains("· current"));
    console.input.write_all(b"/unknown").unwrap();
    wait_for(|| console.output().contains("No matches"));
    let narrow = snapshot(&fixture.0, "narrow");
    println!("{narrow}");
    assert!(narrow.contains("gpt-5.6-terra"));
    console.input.write_all(b"\x1b").unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let closed = snapshot(&fixture.0, "closed");
    assert!(!closed.contains("No matches"));
    assert!(
        !closed.contains("Model changed"),
        "old local output returned: {closed}"
    );
    assert!(closed.contains("/unknown"));
    // Reports remain in scrollback across resize; they never occupy the input.
    console.input.write_all(b"\x03/context\r").unwrap();
    wait_for(|| console.output().contains("canonical turns"));
    std::thread::sleep(Duration::from_millis(100));
    let report = snapshot(&fixture.0, "report");
    println!("{report}");
    assert!(report.find("canonical turns").unwrap() < report.find('─').unwrap());
    inside_composer(&report, "Ask anything");
    console.resize(80, 30);
    std::thread::sleep(Duration::from_millis(100));
    let resized = snapshot(&fixture.0, "report-resized");
    inside_composer(&resized, "Ask anything");
    console.input.write_all(b"\x1b").unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let dismissed = snapshot(&fixture.0, "report-retained");
    println!("{dismissed}");
    assert_eq!(
        dismissed.matches("canonical turns").count(),
        1,
        "{dismissed}"
    );
    assert_eq!(dismissed.matches("Retained answer").count(), 1);
    assert_eq!(dismissed.matches("Ask anything").count(), 1);
    console.input.write_all(&[17]).unwrap();
    drop(console);
    assert!(!fixture.0.join(".jecode").exists());
}
