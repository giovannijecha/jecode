use super::*;

type Assignments = Vec<(String, String)>;

impl Shell {
    pub(super) fn simple(&mut self, simple: &Simple, io: &Io) -> i32 {
        self.substitution.set(None);
        let (args, local_env) = match self.prepare(simple, io) {
            Ok(prepared) => prepared,
            Err(error) => {
                io.report(&error);
                return 1;
            }
        };
        let redirected;
        let io = if simple.redirects.is_empty() {
            io
        } else {
            match self.redirected(io, &simple.redirects) {
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
        if args.is_empty() {
            for (name, value) in local_env {
                self.variables.set(&name, value);
            }
            return self.substitution.get().unwrap_or(0);
        }
        if builtins::handles(&args) {
            // Assignments before a built-in last only for that command.
            let saved: Vec<_> = local_env
                .into_iter()
                .map(|(name, value)| {
                    let previous = self.variables.get(&name).map(str::to_owned);
                    self.variables.set(&name, value);
                    (name, previous)
                })
                .collect();
            let result = builtins::run(self, &args, io);
            for (name, previous) in saved.into_iter().rev() {
                match previous {
                    Some(value) => self.variables.set(&name, value),
                    None => self.variables.remove(&name),
                }
            }
            let (status, stdout, stderr) = match result {
                Ok(result) => result,
                Err(error) => {
                    io.report(&error);
                    return 1;
                }
            };
            for (sink, data) in [(&io.stdout, stdout), (&io.stderr, stderr)] {
                if data.is_empty() {
                    continue;
                }
                match sink.write_all(&data) {
                    Ok(()) => {}
                    Err(error) if streams::closed(&error) => {
                        self.broken_output = true;
                        return 1;
                    }
                    Err(error) => {
                        io.report(&error.to_string());
                        return 1;
                    }
                }
            }
            return status;
        }
        self.external(&args, &local_env, io)
            .unwrap_or_else(|error| {
                io.report(&error);
                127
            })
    }

    /// Expands a simple command's words and separates leading `NAME=value`
    /// assignments, which apply to the command's environment only.
    pub(super) fn prepare(
        &self,
        simple: &Simple,
        io: &Io,
    ) -> Result<(Vec<String>, Assignments), String> {
        let mut args = self.words(&simple.words, io)?;
        let count = args.iter().take_while(|arg| assignment(arg)).count();
        let local_env = args
            .drain(..count)
            .map(|arg| {
                let (name, value) = arg.split_once('=').unwrap();
                (name.to_owned(), value.to_owned())
            })
            .collect();
        Ok((args, local_env))
    }

    /// Applies redirections in order to a copy of `io`.
    pub(super) fn redirected(&self, io: &Io, redirects: &[Redirect]) -> Result<Io, String> {
        let mut io = io.try_clone()?;
        for redirect in redirects {
            if redirect.mode == ">&2" {
                io.stdout = io.stderr.try_clone().map_err(|error| error.to_string())?;
                continue;
            }
            let target = self.expand(&redirect.target, &io)?;
            if redirect.mode == "<" {
                io.stdin = if target == "/dev/null" {
                    Source::Null
                } else {
                    let path = self.path(&target);
                    fs::File::open(&path)
                        .map(Source::File)
                        .map_err(|error| format!("{}: {error}", path.display()))?
                };
                continue;
            }
            let sink = if target == "/dev/null" {
                Sink::Null
            } else {
                let path = self.path(&target);
                OpenOptions::new()
                    .write(true)
                    .create(true)
                    .append(redirect.mode == ">>")
                    .truncate(redirect.mode == ">")
                    .open(&path)
                    .map(Sink::File)
                    .map_err(|error| format!("{}: {error}", path.display()))?
            };
            if redirect.stream == 2 {
                io.stderr = sink;
            } else {
                io.stdout = sink;
            }
        }
        Ok(io)
    }

    fn external(
        &mut self,
        args: &[String],
        local_env: &[(String, String)],
        io: &Io,
    ) -> Result<i32, String> {
        let mut command = self.program(args);
        self.attach(&mut command, io, local_env)?;
        // Only an external stage writing to a pipeline needs a relay. It lets
        // this shell observe an actual closed downstream pipe while preserving
        // the child's direct stdin and stderr attachments.
        let relay_sink = if matches!(&io.stdout, Sink::Pipe(_)) {
            command.stdout(Stdio::piped());
            Some(io.stdout.try_clone().map_err(|error| error.to_string())?)
        } else {
            None
        };
        let mut child = command
            .spawn()
            .map_err(|error| format!("{}: {error}", args[0]))?;
        drop(command);
        let relay = relay_sink.map(|sink| {
            let mut stdout = child.stdout.take().expect("piped external stdout");
            std::thread::spawn(move || {
                let mut writer = &sink;
                io::copy(&mut stdout, &mut writer)
            })
        });
        let status = child.wait();
        // EOF can arrive after the direct child exits if a descendant inherited
        // its stdout. The relay stays until that real writer closes.
        if let Some(result) = relay {
            match result.join().map_err(|_| "stdout relay panicked")? {
                Ok(_) => {}
                Err(error) if streams::closed(&error) => {
                    self.broken_output = true;
                    return Ok(1);
                }
                Err(error) => return Err(format!("stdout relay: {error}")),
            }
        }
        let status = status.map_err(|error| error.to_string())?;
        Ok(status.code().unwrap_or(1))
    }

    /// Gives a child process this shell's directory, variables and streams.
    pub(super) fn attach(
        &self,
        command: &mut Command,
        io: &Io,
        local_env: &[(String, String)],
    ) -> Result<(), String> {
        let stdio = || -> io::Result<(Stdio, Stdio, Stdio)> {
            Ok((io.stdin.stdio()?, io.stdout.stdio(1)?, io.stderr.stdio(2)?))
        };
        let (stdin, stdout, stderr) = stdio().map_err(|error| error.to_string())?;
        command
            .current_dir(&self.cwd)
            .stdin(stdin)
            .stdout(stdout)
            .stderr(stderr);
        for (name, value) in self.variables.iter() {
            command.env(name, value);
        }
        for (name, value) in local_env {
            command.env(name, value);
        }
        Ok(())
    }

    pub(super) fn program(&self, args: &[String]) -> Command {
        let name = Path::new(&args[0]);
        let executable = if !name.is_absolute() && name.components().count() > 1 {
            self.cwd.join(name)
        } else {
            name.to_path_buf()
        };
        let mut command = Command::new(executable);
        command.args(&args[1..]);
        command
    }
}
