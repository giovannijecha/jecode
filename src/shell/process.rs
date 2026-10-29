use super::*;

impl Shell {
    pub(super) fn external(
        &self,
        args: &[String],
        input: Option<&[u8]>,
        local_env: &[(String, String)],
        redirects: &[Redirect],
    ) -> Result<(i32, Vec<u8>, Vec<u8>), String> {
        let mut command = self.program(args);
        command
            .current_dir(&self.cwd)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::inherit()
            })
            .stdout(self.stream(redirects, 1)?)
            .stderr(self.stream(redirects, 2)?);
        for (name, value) in &self.variables {
            command.env(name, value);
        }
        for (name, value) in local_env {
            command.env(name, value);
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("{}: {error}", args[0]))?;
        let writer = input.and_then(|data| {
            child.stdin.take().map(|mut stdin| {
                let data = data.to_vec();
                std::thread::spawn(move || stdin.write_all(&data))
            })
        });
        let status = child.wait().map_err(|error| error.to_string())?;
        if let Some(writer) = writer {
            let _ = writer.join();
        }
        Ok((status.code().unwrap_or(1), Vec::new(), Vec::new()))
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

    pub(super) fn stream(&self, redirects: &[Redirect], stream: u8) -> Result<Stdio, String> {
        let Some(redirect) = redirects
            .iter()
            .rev()
            .find(|r| r.stream == stream && r.mode != "<")
        else {
            return Ok(Stdio::inherit());
        };
        if redirect.mode == ">&2" {
            return Err(">&2 is not supported for external programs".into());
        }
        let path = self.path(&self.expand(&redirect.target)?);
        OpenOptions::new()
            .write(true)
            .create(true)
            .append(redirect.mode == ">>")
            .truncate(redirect.mode == ">")
            .open(&path)
            .map(Stdio::from)
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    pub(super) fn streaming_pipeline(&self, stages: &[Simple]) -> Result<i32, String> {
        let mut children: Vec<std::process::Child> = Vec::new();
        let mut upstream = None;
        for (index, stage) in stages.iter().enumerate() {
            match self.spawn_stage(stage, upstream.take(), index + 1 == stages.len()) {
                Ok(mut child) => {
                    upstream = child.stdout.take();
                    children.push(child);
                }
                Err(error) => {
                    for child in &mut children {
                        let _ = child.kill();
                    }
                    for child in &mut children {
                        let _ = child.wait();
                    }
                    return Err(error);
                }
            }
        }
        let mut last = 0;
        for mut child in children {
            last = child
                .wait()
                .map_err(|error| error.to_string())?
                .code()
                .unwrap_or(1);
        }
        Ok(last)
    }

    fn spawn_stage(
        &self,
        stage: &Simple,
        upstream: Option<std::process::ChildStdout>,
        last: bool,
    ) -> Result<std::process::Child, String> {
        let args: Vec<String> = stage
            .words
            .iter()
            .map(|word| self.expand(word))
            .collect::<Result<_, _>>()?;
        if args.is_empty() {
            return Err("empty pipeline stage".into());
        }
        let mut command = if builtins::is_builtin(&args[0]) {
            let mut command = Command::new(executable()?);
            let script = args
                .iter()
                .map(|arg| format!("'{}'", arg.replace('\'', "'\\''")))
                .collect::<Vec<_>>()
                .join(" ");
            command.args([ENTRY, "-c", &script]);
            command
        } else {
            self.program(&args)
        };
        command.current_dir(&self.cwd);
        for (name, value) in &self.variables {
            command.env(name, value);
        }
        let input = if let Some(redirect) = stage.redirects.iter().rev().find(|r| r.mode == "<") {
            let path = self.path(&self.expand(&redirect.target)?);
            Stdio::from(
                fs::File::open(&path).map_err(|error| format!("{}: {error}", path.display()))?,
            )
        } else if let Some(upstream) = upstream {
            Stdio::from(upstream)
        } else {
            Stdio::inherit()
        };
        command.stdin(input);
        if last {
            command.stdout(self.stream(&stage.redirects, 1)?);
        } else {
            command.stdout(Stdio::piped());
        }
        command.stderr(self.stream(&stage.redirects, 2)?);
        command
            .spawn()
            .map_err(|error| format!("{}: {error}", args[0]))
    }
}
