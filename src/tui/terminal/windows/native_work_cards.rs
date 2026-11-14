//! The approved WORK matrix in a real 120-column Windows console.

use super::{Terminal, native_pty_source, native_script, probe};
use crate::{
    test_support::Directory,
    tui::{
        line::Line, render::Renderer, specimen::work_catalog, test_screen::Screen, text, theme,
        view::Frame,
    },
};
use std::{
    io::{self, Write},
    time::Duration,
};

const WIDTH: usize = 120;
const HEIGHT: usize = 40;

#[test]
#[ignore = "requires Windows ConPTY and JECODE_WORK_INPUT for native WORK cards"]
fn native_windows_work_cards() {
    let input = std::env::var_os("JECODE_WORK_INPUT")
        .expect("Set JECODE_WORK_INPUT to the synthetic work matrix JSON");
    assert!(
        std::path::Path::new(&input).is_absolute(),
        "JECODE_WORK_INPUT must be absolute because the ConPTY child uses an isolated directory"
    );
    assert!(
        std::path::Path::new(&input).is_file(),
        "WORK matrix input is missing"
    );
    let directory = Directory::new();
    let executable = std::env::current_exe().unwrap();
    let source = native_pty_source();
    let marker = "    public void Dispose()";
    assert!(
        source.contains(marker),
        "ConPTY host injection anchor changed"
    );
    let source = source.replace(
        marker,
        &format!("{}\n{marker}", include_str!("native_work_cards.cs")),
    );
    let result = native_script::pty(
        source,
        "WorkCards",
        &[
            &executable.to_string_lossy(),
            &directory.path().to_string_lossy(),
            "work-cards",
        ],
        &directory,
        Duration::from_secs(600),
        false,
    );
    assert_eq!(
        result.exit_code,
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&result.stderr),
        String::from_utf8_lossy(&result.stdout)
    );
    assert!(!result.timed_out, "ConPTY WORK matrix timed out");
    let summary = String::from_utf8_lossy(&result.stdout);
    assert!(summary.contains("NATIVE_WORK cases="), "{summary}");
    println!("{}", summary.trim());
}

pub(super) fn console() {
    let cases = work_catalog::cases();
    assert!(!cases.is_empty(), "The native WORK matrix is empty");
    let terminal = Terminal::open().unwrap();
    let mut renderer = Renderer::new();
    let mut pages = 0;
    for case in &cases {
        let columns = case
            .get("cols")
            .and_then(crate::json::Value::as_usize)
            .unwrap();
        assert!(matches!(columns, 40 | 80 | 120));
        theme::set_light(case.get("light") == Some(&crate::json::Value::Bool(true)));
        let rows = work_catalog::rows(case);
        if rows.is_empty() {
            paint(&mut renderer, &[], columns);
            pages += 1;
        } else {
            for page in rows.chunks(HEIGHT) {
                paint(&mut renderer, page, columns);
                pages += 1;
            }
        }
    }
    theme::set_light(false);
    drop(terminal);
    println!("NATIVE_WORK cases={} pages={pages}", cases.len());
}

fn paint(renderer: &mut Renderer, rows: &[Line], columns: usize) {
    assert!(rows.len() <= HEIGHT);
    let expected: String = (0..HEIGHT)
        .map(|index| {
            let value = rows.get(index).map_or_else(String::new, Line::plain);
            assert!(text::cells(&value) <= columns);
            format!("{value}{}", " ".repeat(WIDTH - text::cells(&value)))
        })
        .collect();
    let frame = Frame {
        header: vec![],
        history: vec![],
        live: rows.to_vec(),
        cursor: None,
        composer: HEIGHT - 1,
    };
    renderer.repaint();
    let output = renderer.paint(&frame, WIDTH, HEIGHT);
    let mut screen = Screen::new(WIDTH, HEIGHT);
    screen.feed(&output);
    for (index, line) in rows.iter().enumerate() {
        if line.background.is_some() {
            continue;
        }
        let mut column = 0;
        for span in &line.spans {
            let end = (column + text::cells(&span.text)).min(WIDTH);
            if span.background.is_none() && !span.style.has_fill() {
                assert!(screen.backs[index][column..end].iter().all(Option::is_none));
            }
            column = end;
        }
        assert!(screen.backs[index][column..].iter().all(Option::is_none));
    }
    assert!(
        screen.backs[rows.len()..]
            .iter()
            .flatten()
            .all(Option::is_none)
    );
    write!(io::stdout(), "{output}").unwrap();
    io::stdout().flush().unwrap();
    assert_eq!(probe().lines().nth(1).unwrap(), expected);
}
