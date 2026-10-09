use std::ffi::OsString;

pub enum Action {
    Help,
    Version,
    Setup,
    Resume {
        id: Option<String>,
        plain: bool,
    },
    Run {
        model: Option<String>,
        prompt: Option<String>,
        attachments: Vec<String>,
        plain: bool,
    },
}

pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Action, String> {
    let arguments: Vec<String> = arguments
        .into_iter()
        .map(|value| {
            value
                .into_string()
                .map_err(|_| "Arguments must be valid UTF-8".to_string())
        })
        .collect::<Result<_, _>>()?;
    if arguments.len() == 1 {
        match arguments[0].as_str() {
            "--help" | "-h" => return Ok(Action::Help),
            "--version" | "-V" => return Ok(Action::Version),
            "setup" => return Ok(Action::Setup),
            _ => {}
        }
    }
    let mut arguments = arguments.into_iter();
    let mut model = None;
    let mut prompt = Vec::new();
    let mut plain = false;
    let mut resume = false;
    let mut attachments = Vec::new();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--plain" if prompt.is_empty() => plain = true,
            "resume" if prompt.is_empty() && !resume => resume = true,
            "--model" if prompt.is_empty() => {
                if model.is_some() {
                    return Err("--model may only be supplied once".into());
                }
                let value = arguments
                    .next()
                    .ok_or("--model requires a model identifier")?;
                if value.trim().is_empty() || value.starts_with('-') {
                    return Err("--model requires a model identifier".into());
                }
                model = Some(value);
            }
            "--attach" if prompt.is_empty() => {
                let value = arguments.next().ok_or("--attach requires a file path")?;
                if value.is_empty() {
                    return Err("--attach requires a file path".into());
                }
                attachments.push(value);
            }
            "--" => {
                prompt.extend(arguments);
                break;
            }
            value if value.starts_with('-') => {
                return Err(format!("Unknown argument: {value}. Use --help for usage."));
            }
            _ => prompt.push(argument),
        }
    }
    if resume {
        if model.is_some() || prompt.len() > 1 || !attachments.is_empty() {
            return Err("Usage: jecode [--plain] resume [SESSION_ID]. Change the model with /model after resuming.".into());
        }
        return Ok(Action::Resume {
            id: prompt.pop(),
            plain,
        });
    }
    let prompt = if prompt.is_empty() {
        None
    } else {
        Some(prompt.join(" "))
    };
    if prompt
        .as_ref()
        .is_some_and(|prompt| prompt.trim().is_empty())
    {
        return Err("The prompt must not be empty".into());
    }
    Ok(Action::Run {
        model,
        prompt,
        attachments,
        plain,
    })
}

pub fn print_help() {
    let interaction = "Without PROMPT, open the fullscreen TUI or read one task from piped stdin.\n--plain uses a line-based chat. TUI: Ctrl+Q quits; Ctrl+C stops work.\nWheel and PgUp/PgDn scroll the conversation; Up/Down browse prompt history.";
    #[cfg(windows)]
    let requirements = "Requires Windows 10+, HTTPS-enabled curl, Git Bash and installed Windows PowerShell/.NET.\nThe Windows TUI also uses console APIs.";
    #[cfg(not(windows))]
    let requirements = "Requires HTTPS-enabled curl, Bash and kill on PATH.\nThe Unix TUI also requires stty and a VT terminal with /dev/tty.";
    println!(
        "Jecode\n\nUsage: jecode [--plain] [--model MODEL] [--attach PATH]... [PROMPT]\n       jecode [--plain] resume [SESSION_ID]\n       jecode setup\n       jecode --help\n       jecode --version\n\nStart from your project directory. First launch guides you through setup.\nKey, default model and effort live in plain text in ~/.jecode/config.json.\nConversations autosave in ~/.jecode/sessions/ and resume only in their original folder.\n--model overrides the model for this run without changing saved defaults.\n--attach adds a file to the first message; repeat it for more files.\n\n{interaction}\nOptions precede PROMPT; use -- before a prompt starting with '-'.\n\nTools: read, write, edit, bash (direct execution).\n\n{}\n\n{requirements}\n\nAdvanced: OPENROUTER_API_KEY and OPENROUTER_MODEL apply when no config exists.\nJECODE_HOME overrides the config directory; JECODE_BASH selects Bash.",
        crate::session::HELP,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn resume_is_explicit_and_plain_mode_remains_available() {
        assert!(matches!(
            parse(arguments(&["resume"])).unwrap(),
            Action::Resume {
                id: None,
                plain: false
            }
        ));
        assert!(
            matches!(parse(arguments(&["--plain", "resume", "123-456"])).unwrap(), Action::Resume { id: Some(id), plain: true } if id == "123-456")
        );
        assert!(parse(arguments(&["resume", "a", "b"])).is_err());
        assert!(parse(arguments(&["--model", "fixture/model", "resume"])).is_err());
        assert!(
            matches!(parse(arguments(&["--", "resume"])).unwrap(), Action::Run { prompt: Some(prompt), .. } if prompt == "resume")
        );
    }

    #[test]
    fn attach_is_repeatable_and_precedes_the_prompt() {
        assert!(matches!(
            parse(arguments(&["--attach", "a.png", "--plain", "--attach", "b c.pdf", "explain"])).unwrap(),
            Action::Run { attachments, prompt: Some(prompt), plain: true, .. }
                if attachments == ["a.png", "b c.pdf"] && prompt == "explain"
        ));
        assert!(parse(arguments(&["explain", "--attach", "a.png"])).is_err());
        assert!(parse(arguments(&["--attach"])).is_err());
        assert!(parse(arguments(&["--attach", ""])).is_err());
        assert!(parse(arguments(&["--attach", "a.png", "resume"])).is_err());
    }

    #[test]
    fn selects_model_and_preserves_prompt() {
        match parse(arguments(&[
            "--model",
            "provider/model",
            "Explain",
            "this project",
        ]))
        .unwrap()
        {
            Action::Run { model, prompt, .. } => {
                assert_eq!(model.as_deref(), Some("provider/model"));
                assert_eq!(prompt.as_deref(), Some("Explain this project"));
            }
            _ => panic!("expected a run action"),
        }
        assert!(matches!(
            parse(arguments(&["--help"])).unwrap(),
            Action::Help
        ));
        assert!(matches!(
            parse(arguments(&["setup"])).unwrap(),
            Action::Setup
        ));
        assert!(matches!(
            parse(arguments(&[])).unwrap(),
            Action::Run { prompt: None, .. }
        ));
        assert!(matches!(
            parse(arguments(&["--", "-a prompt"])).unwrap(),
            Action::Run {
                prompt: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn rejects_incomplete_and_conflicting_options() {
        for values in [
            vec!["--model"],
            vec!["--model", "--help"],
            vec!["--model", "a", "--model", "b"],
            vec!["--unknown"],
            vec!["--help", "--version"],
            vec![" "],
        ] {
            assert!(parse(arguments(&values)).is_err());
        }
    }

    #[test]
    fn plain_mode_remains_explicit_and_options_precede_the_prompt() {
        assert!(matches!(
            parse(arguments(&["--plain", "--model", "fixture/model"])).unwrap(),
            Action::Run {
                plain: true,
                prompt: None,
                ..
            }
        ));
        assert!(matches!(
            parse(arguments(&["--plain", "hello"])).unwrap(),
            Action::Run {
                plain: true,
                prompt: Some(_),
                ..
            }
        ));
        assert!(parse(arguments(&["hello", "--plain"])).is_err());
    }
}
