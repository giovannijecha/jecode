use super::*;

impl Shell {
    pub(super) fn if_command(&mut self, tokens: &[Token]) -> i32 {
        let Some(condition_end) = tokens
            .iter()
            .position(|token| matches!(token, Token::Op(";")))
        else {
            eprintln!("shell: if: expected '; then'");
            return 2;
        };
        if !tokens
            .get(condition_end + 1)
            .is_some_and(|token| literal(token, "then"))
        {
            eprintln!("shell: if: expected then");
            return 2;
        }
        let body_start = condition_end + 2;
        let Some(body_end) = (body_start..tokens.len()).find(|index| {
            literal(&tokens[*index], "fi")
                && (*index == body_start || matches!(tokens[*index - 1], Token::Op(";")))
        }) else {
            eprintln!("shell: if: missing fi");
            return 2;
        };
        let alternate = (body_start..body_end).find(|index| {
            literal(&tokens[*index], "else")
                && (*index == body_start || matches!(tokens[*index - 1], Token::Op(";")))
        });
        let passed = self.run_tokens(&tokens[1..condition_end]) == 0;
        let selected = if passed {
            &tokens[body_start..alternate.unwrap_or(body_end)]
        } else if let Some(alternate) = alternate {
            &tokens[alternate + 1..body_end]
        } else {
            &[][..]
        };
        let status = if selected.is_empty() {
            0
        } else {
            self.run_tokens(selected)
        };
        if self.exit.is_some() || self.stop_on_error && status != 0 {
            return status;
        }
        self.status = status;
        let suffix = &tokens[body_end + 1..];
        if suffix.iter().any(|token| !matches!(token, Token::Op(";"))) {
            self.run_tokens(suffix)
        } else {
            status
        }
    }

    fn c_for_loop(&mut self, tokens: &[Token]) -> i32 {
        let invalid = || {
            eprintln!("shell: for: expected 'for ((NAME=START; NAME<LIMIT; NAME++)); do' ");
            2
        };
        if !matches!(tokens.get(2), Some(Token::Op("("))) {
            return invalid();
        }
        let Some(first_end) = tokens
            .iter()
            .position(|token| matches!(token, Token::Op(";")))
        else {
            return invalid();
        };
        let Some(second_end) =
            (first_end + 1..tokens.len()).find(|index| matches!(tokens[*index], Token::Op(";")))
        else {
            return invalid();
        };
        let Some(end) = (second_end + 1..tokens.len().saturating_sub(1)).find(|index| {
            matches!(tokens[*index], Token::Op(")")) && matches!(tokens[*index + 1], Token::Op(")"))
        }) else {
            return invalid();
        };
        let [Token::Word(init)] = &tokens[3..first_end] else {
            return invalid();
        };
        let init = match self.expand(init) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("shell: {error}");
                return 1;
            }
        };
        let Some((name, start)) = init.split_once('=') else {
            return invalid();
        };
        if !assignment(&init) {
            return invalid();
        }
        let Ok(start) = start.parse::<i64>() else {
            return invalid();
        };
        let [
            Token::Word(condition_name),
            Token::Op("<"),
            Token::Word(limit),
        ] = &tokens[first_end + 1..second_end]
        else {
            return invalid();
        };
        if self.expand(condition_name).ok().as_deref() != Some(name) {
            return invalid();
        }
        let Ok(limit) = self.expand(limit).unwrap_or_default().parse::<i64>() else {
            return invalid();
        };
        let [Token::Word(increment)] = &tokens[second_end + 1..end] else {
            return invalid();
        };
        if self.expand(increment).ok().as_deref() != Some(&format!("{name}++")) {
            return invalid();
        }
        if !matches!(tokens.get(end + 2), Some(Token::Op(";")))
            || !tokens
                .get(end + 3)
                .is_some_and(|token| literal(token, "do"))
        {
            return invalid();
        }
        let body_start = end + 4;
        let Some(body_end) = (body_start..tokens.len()).find(|index| {
            literal(&tokens[*index], "done")
                && (*index == body_start || matches!(tokens[*index - 1], Token::Op(";")))
        }) else {
            return invalid();
        };
        if (limit as i128 - start as i128).abs() > 1_000_000 {
            eprintln!("shell: for: iteration limit exceeded");
            return 2;
        }
        let mut status = 0;
        for value in start..limit {
            self.variables.insert(name.into(), value.to_string());
            status = self.run_tokens(&tokens[body_start..body_end]);
            if self.exit.is_some() || self.stop_on_error && status != 0 {
                return status;
            }
        }
        self.status = status;
        let suffix = &tokens[body_end + 1..];
        if suffix.iter().any(|token| !matches!(token, Token::Op(";"))) {
            self.run_tokens(suffix)
        } else {
            status
        }
    }

    pub(super) fn loop_commands(&mut self, tokens: &[Token]) -> i32 {
        let until = literal(&tokens[0], "until");
        let Some(condition_end) = tokens
            .iter()
            .position(|token| matches!(token, Token::Op(";")))
        else {
            eprintln!("shell: loop: expected '; do'");
            return 2;
        };
        if !tokens
            .get(condition_end + 1)
            .is_some_and(|token| literal(token, "do"))
        {
            eprintln!("shell: loop: expected do");
            return 2;
        }
        let body_start = condition_end + 2;
        let Some(body_end) = (body_start..tokens.len()).find(|index| {
            literal(&tokens[*index], "done")
                && (*index == body_start || matches!(tokens[*index - 1], Token::Op(";")))
        }) else {
            eprintln!("shell: loop: missing done");
            return 2;
        };
        let condition = &tokens[1..condition_end];
        let body = &tokens[body_start..body_end];
        let mut last = 0;
        loop {
            let status = self.run_tokens(condition);
            if (status == 0) == until {
                break;
            }
            last = self.run_tokens(body);
            if self.exit.is_some() || self.stop_on_error && last != 0 {
                return last;
            }
        }
        self.status = last;
        let suffix = &tokens[body_end + 1..];
        if suffix.iter().any(|token| !matches!(token, Token::Op(";"))) {
            self.run_tokens(suffix)
        } else {
            last
        }
    }

    pub(super) fn background(&mut self, stages: &[Simple]) -> i32 {
        if stages.len() != 1 {
            eprintln!("shell: background pipelines are unsupported");
            return 2;
        }
        let args: Vec<String> = match stages[0]
            .words
            .iter()
            .map(|word| self.expand(word))
            .collect()
        {
            Ok(args) => args,
            Err(error) => {
                eprintln!("shell: {error}");
                return 1;
            }
        };
        if args.is_empty() {
            eprintln!("shell: empty background command");
            return 2;
        }
        let mut command = if builtins::is_builtin(&args[0]) {
            match executable() {
                Ok(path) => {
                    let mut command = Command::new(path);
                    command.args([ENTRY, "-c", &script_args(&args)]);
                    command
                }
                Err(error) => {
                    eprintln!("shell: {error}");
                    return 1;
                }
            }
        } else {
            self.program(&args)
        };
        let redirects = &stages[0].redirects;
        let input = if let Some(redirect) = redirects.iter().rev().find(|r| r.mode == "<") {
            match self.expand(&redirect.target).and_then(|target| {
                fs::File::open(self.path(&target))
                    .map(Stdio::from)
                    .map_err(|e| e.to_string())
            }) {
                Ok(input) => input,
                Err(error) => {
                    eprintln!("shell: {error}");
                    return 1;
                }
            }
        } else {
            Stdio::null()
        };
        let (stdout, stderr) = match (self.stream(redirects, 1), self.stream(redirects, 2)) {
            (Ok(stdout), Ok(stderr)) => (stdout, stderr),
            (Err(error), _) | (_, Err(error)) => {
                eprintln!("shell: {error}");
                return 1;
            }
        };
        command
            .current_dir(&self.cwd)
            .stdin(input)
            .stdout(stdout)
            .stderr(stderr);
        for (name, value) in &self.variables {
            command.env(name, value);
        }
        match command.spawn() {
            Ok(child) => {
                self.last_background = Some(child.id());
                self.jobs.push(child);
                0
            }
            Err(error) => {
                eprintln!("shell: {}: {error}", args[0]);
                127
            }
        }
    }

    pub(super) fn background_subshell(&mut self, tokens: &[Token]) -> i32 {
        let Some(end) = tokens
            .iter()
            .position(|token| matches!(token, Token::Op(")")))
        else {
            eprintln!("shell: missing subshell ')' ");
            return 2;
        };
        if !matches!(tokens.get(end + 1), Some(Token::Op("&"))) {
            eprintln!("shell: only background subshells are supported");
            return 2;
        }
        let script = match tokens[1..end]
            .iter()
            .map(|token| match token {
                Token::Word(word) => self.expand(word).map(|value| shell_quote(&value)),
                Token::Op(op) => Ok(op.to_string()),
            })
            .collect::<Result<Vec<_>, String>>()
        {
            Ok(parts) => parts.join(" "),
            Err(error) => {
                eprintln!("shell: {error}");
                return 1;
            }
        };
        let mut command = match executable() {
            Ok(path) => Command::new(path),
            Err(error) => {
                eprintln!("shell: {error}");
                return 1;
            }
        };
        command
            .args([ENTRY, "-c", &script])
            .current_dir(&self.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        for (name, value) in &self.variables {
            command.env(name, value);
        }
        match command.spawn() {
            Ok(child) => {
                self.last_background = Some(child.id());
                self.jobs.push(child);
                self.status = 0;
            }
            Err(error) => {
                eprintln!("shell: subshell: {error}");
                return 127;
            }
        }
        self.run_tokens(&tokens[end + 2..])
    }

    pub(super) fn for_loop(&mut self, tokens: &[Token]) -> i32 {
        if matches!(tokens.get(1), Some(Token::Op("("))) {
            return self.c_for_loop(tokens);
        }
        let header_end = tokens
            .iter()
            .position(|token| matches!(token, Token::Op(";")));
        let Some(header_end) = header_end else {
            eprintln!("shell: for: expected '; do'");
            return 2;
        };
        let header = &tokens[..header_end];
        if header.len() < 4
            || !literal(&header[2], "in")
            || !literal(tokens.get(header_end + 1).unwrap_or(&Token::Op(";")), "do")
        {
            eprintln!("shell: for: expected 'for NAME in WORDS; do'");
            return 2;
        }
        let Token::Word(variable) = &header[1] else {
            unreachable!()
        };
        let name = match self.expand(variable) {
            Ok(name) => name,
            Err(error) => {
                eprintln!("shell: {error}");
                return 1;
            }
        };
        if !assignment(&format!("{name}=")) {
            eprintln!("shell: for: invalid variable name");
            return 2;
        }
        let body_start = header_end + 2;
        let Some(body_end) = (body_start..tokens.len()).find(|index| {
            literal(&tokens[*index], "done")
                && (*index == body_start || matches!(tokens[*index - 1], Token::Op(";")))
        }) else {
            eprintln!("shell: for: missing done");
            return 2;
        };
        let body = &tokens[body_start..body_end];
        let mut result = 0;
        for value in &header[3..] {
            let Token::Word(value) = value else {
                eprintln!("shell: for: invalid value list");
                return 2;
            };
            let value = match self.expand(value) {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("shell: {error}");
                    return 1;
                }
            };
            self.variables.insert(name.clone(), value);
            result = self.run_tokens(body);
            if self.exit.is_some() || self.stop_on_error && result != 0 {
                return result;
            }
        }
        self.status = result;
        let suffix = &tokens[body_end + 1..];
        if suffix.iter().any(|token| !matches!(token, Token::Op(";"))) {
            self.run_tokens(suffix)
        } else {
            result
        }
    }
}

fn shell_quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

fn script_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| shell_quote(arg))
        .collect::<Vec<_>>()
        .join(" ")
}
