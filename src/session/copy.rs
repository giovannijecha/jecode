use crate::{agent::Agent, clipboard::Delivery, copy::Target, export::Archive};
use std::io::{self, BufRead, IsTerminal, Write};

pub(super) fn print(
    agent: &Agent,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<(), String> {
    let Some(target) = select(&agent.archive(), input, output)? else {
        return Ok(());
    };
    let terminal = io::stdout().is_terminal();
    match crate::clipboard::copy(
        &target.text,
        terminal,
        &crate::cancel::Cancellation::default(),
    )? {
        Delivery::Confirmed => writeln!(
            output,
            "Copied {} to clipboard",
            crate::copy::preview(&target.name)
        ),
        Delivery::Terminal(packet) => output
            .write_all(packet.as_bytes())
            .and_then(|()| writeln!(output, "Copy sent to terminal; confirmation unavailable")),
    }
    .and_then(|()| output.flush())
    .map_err(|error| error.to_string())
}

fn select(
    archive: &Archive,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<Option<Target>, String> {
    let mut targets = crate::copy::response(archive)?;
    writeln!(output, "Copy to clipboard").map_err(|error| error.to_string())?;
    for (index, target) in targets.iter().enumerate() {
        writeln!(
            output,
            "{}. {} — {}",
            index + 1,
            crate::copy::preview(&target.name),
            crate::copy::preview(&target.text)
        )
        .map_err(|error| error.to_string())?;
    }
    write!(output, "Choice [Enter = 1, /cancel = close]: ")
        .and_then(|()| output.flush())
        .map_err(|error| error.to_string())?;
    let Some(choice) = crate::input::read_line(input)? else {
        return Ok(None);
    };
    let choice = choice.trim();
    if matches!(choice, "/cancel" | "0") {
        return Ok(None);
    }
    let index = if choice.is_empty() {
        1
    } else {
        choice
            .parse::<usize>()
            .map_err(|_| "Choose a listed number or /cancel")?
    };
    if index == 0 || index > targets.len() {
        return Err("Choose a listed number or /cancel".into());
    }
    Ok(Some(targets.swap_remove(index - 1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{json::Value, redact::Redactor};
    use std::{
        io::Cursor,
        sync::{Arc, Mutex},
    };
    fn archive() -> Archive {
        Archive {
            model: "fixture/model".into(),
            directory: "fixture".into(),
            effort: "default".into(),
            redactor: Redactor::empty(),
            events: Arc::new(Mutex::new(vec![])),
            attachments: None,
            messages: Arc::new(Mutex::new(vec![Value::object([
                ("role", Value::string("assistant")),
                (
                    "content",
                    Value::string("Intro\r\n```\r\n  café  \r\n```\r\n> quote\r\n"),
                ),
            ])])),
        }
    }
    #[test]
    fn numeric_and_default_choices_use_the_exact_original_text() {
        let archive = archive();
        let selected = select(&archive, &mut Cursor::new("2\n"), &mut Vec::new())
            .unwrap()
            .unwrap();
        assert_eq!(selected.text, "  café  \r\n");
        let selected = select(&archive, &mut Cursor::new("\n"), &mut Vec::new())
            .unwrap()
            .unwrap();
        assert!(selected.text.starts_with("Intro\r\n"));
    }
    #[test]
    fn cancellation_eof_and_invalid_choices_do_not_call_a_clipboard_backend() {
        for choice in ["/cancel\n", "0\n", ""] {
            assert!(
                select(&archive(), &mut Cursor::new(choice), &mut Vec::new())
                    .unwrap()
                    .is_none()
            );
        }
        assert!(select(&archive(), &mut Cursor::new("99\n"), &mut Vec::new()).is_err());
    }

    #[test]
    fn plain_chat_opens_copy_without_an_extra_request_and_accepts_cancel() {
        use crate::{
            config::{Settings, Store},
            openrouter::OpenRouter,
            session::{SessionConfig, chat},
            test_support::{Directory, HttpFixture, completion},
            tools::Tools,
        };
        let directory = Directory::new();
        let fixture = HttpFixture::new(vec![(
            200,
            completion("Reply\n```rust\n  exact\n```", vec![]),
        )]);
        let mut agent = Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        );
        let mut config = SessionConfig {
            store: Store::new(directory.path().join(".jecode")),
            settings: Settings::new("fixture-key".into(), "fixture/model".into()).unwrap(),
            bash: crate::tools::find_bash().unwrap(),
        };
        let mut output = Vec::new();
        chat(
            &mut agent,
            &mut config,
            &mut Cursor::new("Task\n/copy\n/cancel\n/exit\n"),
            &mut output,
            &mut Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Copy to clipboard"));
        assert!(output.contains("2. Code block 1 · rust"));
        assert_eq!(fixture.finish().len(), 1);
        assert!(agent.archive().events.lock().unwrap().iter().all(|event| {
            event.get("command").and_then(crate::json::Value::as_str) == Some("Report delivery")
                && event.get("receipt").and_then(crate::json::Value::as_str)
                    == Some("report_delivery")
        }));
    }
}
