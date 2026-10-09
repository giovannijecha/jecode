use super::Prompter;
use crate::input::read_line;
use std::io::{BufRead, Write};
use std::path::Path;
use std::process::{Command, Stdio};

pub struct Terminal<'a, R, W> {
    input: &'a mut R,
    output: &'a mut W,
    bash: &'a Path,
}

impl<'a, R: BufRead, W: Write> Terminal<'a, R, W> {
    pub fn new(input: &'a mut R, output: &'a mut W, bash: &'a Path) -> Self {
        Self {
            input,
            output,
            bash,
        }
    }
}

impl<R: BufRead, W: Write> Prompter for Terminal<'_, R, W> {
    fn message(&mut self, text: &str) -> Result<(), String> {
        writeln!(self.output, "{text}")
            .and_then(|_| self.output.flush())
            .map_err(|error| error.to_string())
    }

    fn input(&mut self, prompt: &str) -> Result<Option<String>, String> {
        write!(self.output, "{prompt}")
            .and_then(|_| self.output.flush())
            .map_err(|error| error.to_string())?;
        read_line(self.input)
    }

    fn secret(&mut self, prompt: &str) -> Result<Option<String>, String> {
        write!(self.output, "{prompt}")
            .and_then(|_| self.output.flush())
            .map_err(|error| error.to_string())?;
        // Bash's native terminal input hides the key; it returns through a private stdout pipe.
        // Interactive stdin stays attached. No credential is included in a command argument.
        let script = "IFS= read -r -s credential || exit 2; printf '\\n' >&2; if (( ${#credential} > 512 )); then exit 3; fi; printf '%s' \"$credential\"";
        let output = Command::new(self.bash)
            .args(["--noprofile", "--norc", "-c", script])
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("BASH_ENV")
            .env_remove("ENV")
            .stdin(Stdio::inherit())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .output()
            .map_err(|error| format!("Could not read the key using Bash: {error}"))?;
        match output.status.code() {
            Some(0) => String::from_utf8(output.stdout)
                .map(Some)
                .map_err(|_| "The key must be valid UTF-8".into()),
            Some(2) | Some(130) => Ok(None),
            Some(3) => Err("The API key exceeds the 512-character limit".into()),
            _ => Err("Could not read the key from the terminal".into()),
        }
    }
}
