//! `/attach` in the line-based chat: files or saved references for the next message.

use crate::{
    agent::Agent,
    attachments::{Attachment, paths},
};
use std::io::Write;

/// The path list of an `/attach` line.
pub fn command(line: &str) -> Option<&str> {
    let (name, arguments) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    name.eq_ignore_ascii_case("/attach").then_some(arguments)
}

pub fn stage(
    agent: &Agent,
    arguments: &str,
    staged: &mut Vec<Attachment>,
    output: &mut impl Write,
) -> Result<(), String> {
    let directory = std::env::current_dir().map_err(|error| error.to_string())?;
    let imported = if arguments.contains("attachment:") {
        let pool = agent
            .sessions()
            .ok_or("Attachment storage is unavailable")?
            .store()
            .attachments();
        paths::words(arguments)?
            .iter()
            .map(|word| {
                if let Some(id) = word.strip_prefix("attachment:") {
                    pool.load(id).map(|stored| stored.attachment)
                } else {
                    pool.import_file(
                        &paths::argument(word, &directory),
                        &crate::cancel::Cancellation::default(),
                    )
                }
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        agent.import_attachments(&paths::typed(arguments, &directory)?)?
    };
    staged.extend(imported);
    announce(staged, output)
}

pub fn announce(staged: &[Attachment], output: &mut impl Write) -> Result<(), String> {
    let labels = staged
        .iter()
        .enumerate()
        .map(|(index, attachment)| attachment.label(index + 1))
        .collect::<Vec<_>>()
        .join(" ");
    writeln!(
        output,
        "Staged {labels} for your next message. Press Enter alone to send only the attachments."
    )
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::{
        config::{Settings, Store},
        json::Value,
        openrouter::OpenRouter,
        test_support::{Directory, HttpFixture, completion},
        tools::Tools,
    };
    use std::io::Cursor;

    #[test]
    fn plain_chat_stages_files_until_the_next_message() {
        let home = Directory::new();
        let project = Directory::new();
        std::fs::write(project.path().join("notes.txt"), "staged text").unwrap();
        let fixture = HttpFixture::new(vec![
            (200, completion("Read.", vec![])),
            (200, completion("Plain.", vec![])),
        ]);
        let mut agent = Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(project.path()).unwrap(),
        );
        let mut config = SessionConfig {
            store: Store::new(home.path().join("config.json")),
            settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
            bash: crate::tools::find_bash().unwrap(),
        };
        let notes = project.path().join("notes.txt");
        let mut input = Cursor::new(format!(
            "/attach missing.txt\n/attach \"{}\"\n\nthen plain\n/exit\n",
            notes.display()
        ));
        let mut output = Vec::new();
        let mut status = Vec::new();
        chat(
            &mut agent,
            &mut config,
            &mut input,
            &mut output,
            &mut status,
            Vec::new(),
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(String::from_utf8(status).unwrap().contains("Cannot attach"));
        assert!(output.contains("Staged [1# File: notes.txt] for your next message."));
        let requests = fixture.finish();
        assert_eq!(requests.len(), 2);
        let last = |index: usize| {
            requests[index]
                .body
                .get("messages")
                .and_then(Value::as_array)
                .unwrap()
                .last()
                .unwrap()
                .encode()
        };
        // Enter alone sends only the staged attachment; the next line is plain.
        assert!(last(0).contains("attachment:att-"));
        assert!(!last(1).contains("attachment:att-"));
    }
}
