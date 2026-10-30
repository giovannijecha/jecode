use super::*;
use parser::{AndOr, Item};

impl Shell {
    pub(super) fn run_list(&mut self, list: &List, io: &Io) -> i32 {
        for item in list {
            if self.exit.is_some() {
                break;
            }
            if let Some(source) = &item.background {
                self.status = self.background(item, source, io);
            } else {
                self.and_or(&item.and_or, io);
            }
        }
        self.status
    }

    fn and_or(&mut self, and_or: &AndOr, io: &Io) {
        self.status = self.pipeline(&and_or.first, io);
        let mut ran_last = and_or.rest.is_empty();
        for (index, (op, pipeline)) in and_or.rest.iter().enumerate() {
            if self.exit.is_some() {
                return;
            }
            ran_last = (*op == "&&") == (self.status == 0);
            if ran_last {
                self.status = self.pipeline(pipeline, io);
                ran_last = index + 1 == and_or.rest.len();
            }
        }
        // `set -e` ignores failures that a later `&&`/`||` handles.
        if self.stop_on_error
            && self.conditions == 0
            && ran_last
            && self.status != 0
            && self.exit.is_none()
        {
            self.exit = Some(self.status);
        }
    }

    fn pipeline(&mut self, stages: &[Node], io: &Io) -> i32 {
        if let [only] = stages {
            return self.node(only, io);
        }
        self.streaming_pipeline(stages, io).unwrap_or_else(|error| {
            io.report(&error);
            1
        })
    }

    /// Runs every stage concurrently in its own subshell, connected by pipes.
    fn streaming_pipeline(&self, stages: &[Node], io: &Io) -> Result<i32, String> {
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            let mut upstream = None;
            for (index, stage) in stages.iter().enumerate() {
                let stdin = match upstream.take() {
                    Some(reader) => Source::Pipe(reader),
                    None => io.stdin.try_clone().map_err(|error| error.to_string())?,
                };
                let stdout = if index + 1 == stages.len() {
                    io.stdout.try_clone().map_err(|error| error.to_string())?
                } else {
                    let (reader, writer) = std::io::pipe().map_err(|error| error.to_string())?;
                    upstream = Some(reader);
                    Sink::Pipe(writer)
                };
                let stage_io = Io {
                    stdin,
                    stdout,
                    stderr: io.stderr.try_clone().map_err(|error| error.to_string())?,
                };
                let mut shell = self.fork();
                handles.push(scope.spawn(move || {
                    let status = shell.node(stage, &stage_io);
                    shell.exit.unwrap_or(status)
                }));
            }
            let mut last = 0;
            for handle in handles {
                last = handle.join().unwrap_or(1);
            }
            Ok(last)
        })
    }

    pub(super) fn node(&mut self, node: &Node, io: &Io) -> i32 {
        let (compound, redirects) = match node {
            Node::Simple(simple) => return self.simple(simple, io),
            Node::Compound(compound, redirects) => (compound, redirects),
        };
        let redirected;
        let io = if redirects.is_empty() {
            io
        } else {
            match self.redirected(io, redirects) {
                Ok(local) => {
                    redirected = local;
                    &redirected
                }
                Err(error) => {
                    io.report(&error);
                    return 1;
                }
            }
        };
        match compound {
            Compound::For { name, values, body } => self.for_loop(name, values, body, io),
            Compound::CountedFor {
                init,
                name,
                limit,
                increment,
                body,
            } => self.counted_for([init, name, limit, increment], body, io),
            Compound::Loop {
                until,
                condition,
                body,
            } => {
                let mut last = 0;
                loop {
                    let status = self.condition(condition, io);
                    if self.exit.is_some() || (status == 0) == *until {
                        break;
                    }
                    last = self.body(body, io);
                    if self.exit.is_some() {
                        break;
                    }
                }
                last
            }
            Compound::If {
                branches,
                otherwise,
            } => {
                for (condition, body) in branches {
                    let status = self.condition(condition, io);
                    if self.exit.is_some() {
                        return status;
                    }
                    if status == 0 {
                        return self.body(body, io);
                    }
                }
                otherwise.as_ref().map_or(0, |body| self.body(body, io))
            }
            Compound::Subshell(list) => {
                let mut shell = self.fork();
                let status = shell.run_list(list, io);
                shell.exit.unwrap_or(status)
            }
        }
    }

    fn condition(&mut self, list: &List, io: &Io) -> i32 {
        self.conditions += 1;
        let status = self.run_list(list, io);
        self.conditions -= 1;
        status
    }

    fn body(&mut self, list: &List, io: &Io) -> i32 {
        if list.is_empty() {
            return 0;
        }
        self.run_list(list, io)
    }

    fn for_loop(&mut self, name: &Word, values: &[Word], body: &List, io: &Io) -> i32 {
        let expanded = self
            .expand(name)
            .and_then(|name| Ok((name, self.words(values)?)));
        let (name, values) = match expanded {
            Ok(expanded) => expanded,
            Err(error) => {
                io.report(&error);
                return 1;
            }
        };
        if !assignment(&format!("{name}=")) {
            io.report("for: invalid variable name");
            return 2;
        }
        let mut status = 0;
        for value in values {
            self.variables.set(&name, value);
            status = self.body(body, io);
            if self.exit.is_some() {
                break;
            }
        }
        status
    }

    fn counted_for(&mut self, header: [&Word; 4], body: &List, io: &Io) -> i32 {
        let [init, name, limit, increment] = header.map(|word| self.expand(word));
        let parsed = (|| {
            let init = init.ok()?;
            let (variable, start) = init.split_once('=')?;
            let start = start.parse::<i64>().ok()?;
            let limit = limit.ok()?.parse::<i64>().ok()?;
            (assignment(&init)
                && name.ok()? == variable
                && increment.ok()? == format!("{variable}++"))
            .then(|| (variable.to_owned(), start, limit))
        })();
        let Some((name, start, limit)) = parsed else {
            io.report(parser::COUNTED_FOR);
            return 2;
        };
        if (limit as i128 - start as i128).abs() > 1_000_000 {
            io.report("for: iteration limit exceeded");
            return 2;
        }
        let mut status = 0;
        for value in start..limit {
            self.variables.set(&name, value.to_string());
            status = self.body(body, io);
            if self.exit.is_some() {
                break;
            }
        }
        status
    }

    fn background(&mut self, item: &Item, source: &str, io: &Io) -> i32 {
        let base = match io.stdout.try_clone().and_then(|stdout| {
            Ok(Io {
                stdin: Source::Null,
                stdout,
                stderr: io.stderr.try_clone()?,
            })
        }) {
            Ok(base) => base,
            Err(error) => {
                io.report(&error.to_string());
                return 1;
            }
        };
        let command = match (&item.and_or.rest[..], &item.and_or.first[..]) {
            ([], [Node::Simple(simple)]) => self.background_simple(simple, &base),
            _ => self.background_script(source, &base).map(Some),
        };
        let mut command = match command {
            Ok(Some(command)) => command,
            Ok(None) => return 0,
            Err(error) => {
                io.report(&error);
                return 1;
            }
        };
        match command.spawn() {
            Ok(child) => {
                self.last_background = Some(child.id());
                self.jobs.push(child);
                0
            }
            Err(error) => {
                io.report(&format!("{}: {error}", command.get_program().display()));
                127
            }
        }
    }

    /// A lone simple command starts directly, so `$!` names the program.
    fn background_simple(&self, simple: &Simple, io: &Io) -> Result<Option<Command>, String> {
        let (args, local_env) = self.prepare(simple)?;
        if args.is_empty() {
            return Ok(None);
        }
        let mut command = if builtins::is_builtin(&args[0]) {
            let mut command = Command::new(executable()?);
            command.args([ENTRY, "-c", &script_args(&args)]);
            command
        } else {
            self.program(&args)
        };
        let io = self.redirected(io, &simple.redirects)?;
        self.attach(&mut command, &io, &local_env)?;
        Ok(Some(command))
    }

    /// Any other background list runs, unexpanded, in a child shell.
    fn background_script(&self, source: &str, io: &Io) -> Result<Command, String> {
        let script = if self.stop_on_error {
            format!("set -e\n{source}")
        } else {
            source.to_owned()
        };
        let mut command = Command::new(executable()?);
        command
            .args([ENTRY, "-c", &script, &self.arg_zero])
            .args(&self.positional);
        self.attach(&mut command, io, &[])?;
        Ok(command)
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
