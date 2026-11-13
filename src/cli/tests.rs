use super::*;

fn parse_args(values: &[&str]) -> Result<Invocation, Usage> {
    parse(values.iter().map(OsString::from))
}

fn run(values: &[&str]) -> Run {
    match parse_args(values) {
        Ok(Invocation {
            action: Action::Run(run),
            ..
        }) => run,
        Ok(_) => panic!("expected a run for {values:?}"),
        Err(error) => panic!("{values:?}: {}", error.message),
    }
}

fn error(values: &[&str]) -> String {
    match parse_args(values) {
        Err(error) => error.message,
        Ok(_) => panic!("expected a usage error for {values:?}"),
    }
}

fn help(values: &[&str]) -> Vec<&'static str> {
    match parse_args(values) {
        Ok(Invocation {
            action: Action::Help(path),
            ..
        }) => path,
        _ => panic!("expected help for {values:?}"),
    }
}

fn manage(values: &[&str]) -> Manage {
    match parse_args(values) {
        Ok(Invocation {
            action: Action::Manage(manage),
            ..
        }) => manage,
        Ok(_) => panic!("expected a management command for {values:?}"),
        Err(error) => panic!("{values:?}: {}", error.message),
    }
}

#[test]
fn default_prompt_and_literal_prompts_are_preserved() {
    let plain = run(&["Explain", "this project"]);
    assert_eq!(plain.prompt.as_deref(), Some("Explain this project"));
    assert_eq!(plain.start, Start::New);
    // A command name inside a quoted prompt, or after --, is prompt text.
    assert_eq!(
        run(&["resume the interrupted migration"]).prompt.as_deref(),
        Some("resume the interrupted migration")
    );
    assert_eq!(
        run(&["--", "resume", "the", "migration"]).prompt.as_deref(),
        Some("resume the migration")
    );
    assert_eq!(
        run(&["--", "-v flag", "--help"]).prompt.as_deref(),
        Some("-v flag --help")
    );
    assert_eq!(
        run(&["fix", "update"]).prompt.as_deref(),
        Some("fix update")
    );
    assert!(run(&[]).prompt.is_none());
    assert!(error(&["explain", "--attach", "a.png"]).contains("follows the prompt"));
    assert!(error(&["update", "the", "readme"]).contains("Quote a prompt"));
    assert!(error(&["   "]).contains("must not be empty"));
}

#[test]
fn run_options_are_validated_before_anything_runs() {
    let chosen = run(&[
        "--provider",
        "anthropic-api",
        "--model=claude-fixture",
        "--routing",
        "auto",
        "--effort",
        "high",
        "--permissions",
        "read",
        "--attach",
        "a.png",
        "--attach",
        "b c.pdf",
        "task",
    ]);
    assert_eq!(
        chosen.selection,
        Selection {
            provider: Some("anthropic-api".into()),
            model: Some("claude-fixture".into()),
            routing: Some(crate::backend::Routing::Auto),
            effort: Some(Effort::High),
            permissions: Some(Level::Read),
        }
    );
    assert_eq!(chosen.attachments, ["a.png", "b c.pdf"]);
    assert!(error(&["--provider", "chatgpt", "task"]).contains("openrouter, openai-api"));
    assert!(error(&["--effort", "extreme", "task"]).contains("invalid --effort"));
    assert!(error(&["--permissions", "admin", "task"]).contains("read, workspace, full"));
    assert!(error(&["--model", "a", "--model", "b", "task"]).contains("only be supplied once"));
    assert!(error(&["--routing", "bad endpoint", "task"]).contains("endpoint tag"));
    assert_eq!(
        run(&["--routing", "deepinfra/fp8", "task"])
            .selection
            .routing,
        Some(crate::backend::Routing::Endpoint("deepinfra/fp8".into()))
    );
    assert!(error(&["--model"]).contains("requires ID"));
    assert!(error(&["--model", "--plain"]).contains("--model=VALUE"));
    assert!(error(&["--json", "--plain", "task"]).contains("cannot be combined"));
    assert!(error(&["--unknown"]).contains("unknown option --unknown"));
    assert!(error(&["--plain=yes"]).contains("does not take a value"));
    assert!(error(&["-x"]).contains("unknown option -x"));
}

#[test]
fn working_directory_applies_to_every_command() {
    let invocation = parse_args(&["-C", "project dir", "sessions", "list"]).unwrap();
    assert_eq!(
        invocation.directory,
        Some(std::path::PathBuf::from("project dir"))
    );
    let invocation = parse_args(&["sessions", "--cd=elsewhere"]).unwrap();
    assert_eq!(
        invocation.directory,
        Some(std::path::PathBuf::from("elsewhere"))
    );
    assert!(error(&["-C"]).contains("requires DIR"));
}

#[test]
fn resume_takes_an_id_or_last_and_an_optional_prompt() {
    let picked = run(&["resume"]);
    assert_eq!(picked.start, Start::Resume(Resume::Pick));
    let plain = run(&["--plain", "resume", "123-456"]);
    assert_eq!(plain.start, Start::Resume(Resume::Id("123-456".into())));
    assert!(plain.plain && plain.prompt.is_none());
    let continued = run(&["resume", "123-456", "--model", "fixture/m", "next step"]);
    assert_eq!(continued.prompt.as_deref(), Some("next step"));
    assert_eq!(continued.selection.model.as_deref(), Some("fixture/m"));
    let last = run(&["resume", "--last", "--", "--flag text"]);
    assert_eq!(last.start, Start::Resume(Resume::Last));
    assert_eq!(last.prompt.as_deref(), Some("--flag text"));
    let attached = run(&[
        "resume", "--last", "--attach", "a.png", "--attach", "b.pdf", "compare",
    ]);
    assert_eq!(attached.attachments, ["a.png", "b.pdf"]);
    assert_eq!(attached.prompt.as_deref(), Some("compare"));
    assert!(error(&["resume", "not-an-id!"]).contains("not a session ID"));
    assert!(error(&["resume", "--json"]).contains("SESSION_ID or --last"));
    assert!(error(&["resume", "1-2", "prompt", "--plain"]).contains("follows the prompt"));
    assert!(error(&["resume", "1-2", "--last"]).contains("cannot combine"));
    assert!(error(&["resume", "1-2", "--last", "prompt"]).contains("cannot combine"));
    assert!(error(&["resume", "prompt", "--last"]).contains("follows the prompt"));
    // With --last the first word already starts the prompt.
    assert!(error(&["resume", "--last", "summarize", "--json"]).contains("follows the prompt"));
    assert!(error(&["init", "extra"]).contains("takes no prompt"));
    assert!(error(&["init", "--attach", "a"]).contains("does not apply"));
    assert_eq!(run(&["init", "--model", "m"]).start, Start::Init);
}

#[test]
fn help_is_contextual_and_never_runs_the_command() {
    assert!(help(&["--help"]).is_empty());
    assert!(help(&["-h"]).is_empty());
    assert_eq!(help(&["resume", "--help"]), ["resume"]);
    assert_eq!(help(&["setup", "--help"]), ["setup"]);
    assert_eq!(
        help(&["sessions", "delete", "123-1", "-h"]),
        ["sessions", "delete"]
    );
    assert_eq!(
        help(&["--model", "sessions", "help-me", "--help"]),
        Vec::<&str>::new()
    );
    assert_eq!(help(&["help", "mcp", "add"]), ["mcp", "add"]);
    assert!(error(&["help", "nonsense"]).contains("unknown command"));
    // After --, --help is prompt text.
    assert_eq!(
        run(&["--", "explain", "--help"]).prompt.as_deref(),
        Some("explain --help")
    );
    for (path, expected) in [
        (vec![], "jecode COMMAND"),
        (vec!["resume"], "--last"),
        (vec!["setup"], "jecode setup"),
        (vec!["settings"], "Keys: model, effort"),
        (vec!["providers"], "providers accounts openai|anthropic"),
        (vec!["providers", "logout"], "--account ACCOUNT_ID"),
        (vec!["providers"], "--yes"),
        (vec!["mcp"], "--auth"),
        (vec!["models", "endpoints"], "models endpoints MODEL"),
        (vec!["update"], "--check"),
    ] {
        let text = help::text(&path);
        assert!(text.contains(expected), "{path:?}: {text}");
    }
}

#[test]
fn commands_reject_inapplicable_options_and_wrong_operands() {
    assert!(error(&["--plain", "setup"]).contains("--plain does not apply to 'jecode setup'"));
    assert!(error(&["--model", "m", "sessions"]).contains("does not apply"));
    assert!(error(&["sessions", "show"]).contains("takes 1 argument"));
    assert!(error(&["sessions", "frobnicate"]).contains("unknown command 'frobnicate'"));
    assert!(error(&["settings", "set", "theme"]).contains("takes 2 arguments"));
    assert!(error(&["completions", "tcsh"]).contains("unsupported shell"));
    assert!(error(&["--version", "extra"]).contains("--version"));
    let listed = manage(&["sessions"]);
    assert_eq!((listed.family, listed.action), ("sessions", "list"));
    let set = manage(&[
        "settings",
        "set",
        "model",
        "fixture/m",
        "--provider",
        "openrouter",
    ]);
    assert_eq!(set.operands, ["model", "fixture/m"]);
    assert_eq!(set.value("provider"), Some("openrouter"));
    assert!(manage(&["--json", "mcp", "list"]).json());
    let accounts = manage(&["providers", "accounts", "openai", "--json"]);
    assert_eq!(
        (accounts.family, accounts.action),
        ("providers", "accounts")
    );
    assert_eq!(accounts.operands, ["openai"]);
    assert!(accounts.json());
    let selected = manage(&["providers", "use", "anthropic", "account-1"]);
    assert_eq!((selected.family, selected.action), ("providers", "use"));
    assert_eq!(selected.operands, ["anthropic", "account-1"]);
    let logout = manage(&[
        "providers",
        "logout",
        "openai",
        "--account",
        "account-1",
        "--yes",
    ]);
    assert_eq!(logout.value("account"), Some("account-1"));
    assert!(logout.flag("yes"));
    let endpoints = manage(&[
        "models",
        "endpoints",
        "fixture/model",
        "--provider",
        "openrouter",
        "--json",
    ]);
    assert_eq!(
        (endpoints.family, endpoints.action),
        ("models", "endpoints")
    );
    assert_eq!(endpoints.operands, ["fixture/model"]);
    assert_eq!(endpoints.value("provider"), Some("openrouter"));
    assert!(endpoints.json());
    assert!(error(&["providers", "accounts"]).contains("takes 1 argument"));
    assert!(error(&["providers", "use", "openai"]).contains("takes 2 arguments"));
    assert!(error(&["providers", "login", "openai", "--account", "x"]).contains("does not apply"));
    assert!(error(&["providers", "logout", "openai", "--account"]).contains("requires ACCOUNT_ID"));
    // `-` alone is a value (stdout), not an option.
    let exported = manage(&["sessions", "export", "1-2", "--output", "-"]);
    assert_eq!(exported.value("output"), Some("-"));
    assert!(error(&["sessions", "export", "1-2", "--output", "-x"]).contains("--output=VALUE"));
    assert!(matches!(
        parse_args(&["update", "--check", "--json"]).unwrap().action,
        Action::Update {
            check: true,
            json: true
        }
    ));
    assert!(matches!(
        parse_args(&["setup"]).unwrap().action,
        Action::Setup
    ));
    assert!(matches!(
        parse_args(&["-V"]).unwrap().action,
        Action::Version
    ));
}

#[test]
fn completions_cover_every_command_and_option_in_each_shell() {
    for shell in grammar::SHELLS {
        let script = completions::script(shell);
        for command in grammar::ROOT.subcommands {
            assert!(script.contains(command.name), "{shell}: {}", command.name);
            for sub in command.subcommands {
                assert!(script.contains(sub.name), "{shell}: {}", sub.name);
            }
        }
        for option in [
            "--provider",
            "--routing",
            "--permissions",
            "--cd",
            "--json",
            "--last",
            "--account",
        ] {
            assert!(
                script.contains(option.trim_start_matches('-')),
                "{shell}: {option}"
            );
        }
    }
    assert!(completions::script("bash").contains("complete -o default -F _jecode jecode"));
    // Free values such as paths complete as files, not command words.
    assert!(completions::script("bash").contains("--attach|"));
    assert!(
        completions::script("zsh").contains("-C|")
            && !completions::script("zsh").contains("local path")
    );
    assert!(completions::script("fish").contains("-s C -l cd -r -F"));
    assert!(completions::script("powershell").contains("$jecodeFree = @('--"));
    assert!(
        grammar::command(&["providers", "logout"])
            .unwrap()
            .allows("account")
    );
    assert!(
        !grammar::command(&["providers", "login"])
            .unwrap()
            .allows("account")
    );
}
