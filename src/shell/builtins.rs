use super::streams::{Io, closed};
use super::{Shell, Variables};
#[path = "builtins/files.rs"]
mod files;
#[path = "builtins/format.rs"]
mod format;
use format::printf;
#[path = "builtins/text.rs"]
mod text;
use files::{copy_move, mktemp, remove};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;
use text::tr;

#[cfg(test)]
pub(super) use files::copy_then_remove;

type Outcome = Result<(i32, Vec<u8>, Vec<u8>), String>;

pub(super) fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        ":" | "true"
            | "false"
            | "exit"
            | "set"
            | "pwd"
            | "cd"
            | "echo"
            | "printf"
            | "cat"
            | "ls"
            | "mkdir"
            | "touch"
            | "rm"
            | "cp"
            | "mv"
            | "test"
            | "["
            | "sleep"
            | "env"
            | "export"
            | "bash"
            | "sh"
            | "wc"
            | "wait"
            | "seq"
            | "mktemp"
            | "tr"
    )
}

/// Whether `args` runs in-process; `env COMMAND` runs the external program.
pub(super) fn handles(args: &[String]) -> bool {
    is_builtin(&args[0]) && !(args[0] == "env" && args.len() > 1)
}

/// Runs a built-in command. Output is either returned or, for commands that
/// move data (`cat`, `tr`, `wc`), streamed through `io`.
pub(super) fn run(shell: &mut Shell, args: &[String], io: &Io) -> Outcome {
    match args[0].as_str() {
        ":" | "true" => ok(Vec::new()),
        "false" => Ok((1, Vec::new(), Vec::new())),
        "exit" => {
            let code = args
                .get(1)
                .and_then(|v| v.parse::<i32>().ok())
                .unwrap_or(shell.status);
            shell.exit = Some(code);
            ok(Vec::new())
        }
        "set" => {
            match args.get(1).map(String::as_str) {
                Some("-e") => shell.stop_on_error = true,
                Some("+e") => shell.stop_on_error = false,
                None => {}
                _ => return Err("set: supported options are -e and +e".into()),
            }
            ok(Vec::new())
        }
        "pwd" => ok(format!("{}\n", shell.cwd.display()).into_bytes()),
        "cd" => {
            let path = args
                .get(1)
                .map(String::as_str)
                .or_else(|| shell.variables.get("HOME"));
            match path {
                Some(path) => {
                    let target = shell.path(path);
                    match target.canonicalize() {
                        Ok(target) if target.is_dir() => {
                            shell.cwd = target;
                            ok(Vec::new())
                        }
                        Ok(_) => Err(format!("cd: {}: not a directory", path)),
                        Err(error) => Err(format!("cd: {path}: {error}")),
                    }
                }
                None => Err("cd: HOME is unset".into()),
            }
        }
        "echo" => {
            let (newline, start) = if args.get(1).is_some_and(|v| v == "-n") {
                (false, 2)
            } else {
                (true, 1)
            };
            let mut text = args[start..].join(" ");
            if newline {
                text.push('\n');
            }
            ok(text.into_bytes())
        }
        "printf" => printf(&args[1..]),
        "cat" => cat(shell, args, io),
        "wc" => {
            if args.get(1).map(String::as_str) != Some("-c") || args.len() != 2 {
                return Err("wc: only -c from stdin is supported".into());
            }
            let count =
                io::copy(&mut &io.stdin, &mut io::sink()).map_err(|e| format!("wc: {e}"))?;
            ok(format!("{count}\n").into_bytes())
        }
        "wait" => {
            if args.len() != 1 {
                return Err("wait: positional job IDs are unsupported".into());
            }
            let mut code = 0;
            for mut job in shell.jobs.drain(..) {
                code = job
                    .wait()
                    .map_err(|e| format!("wait: {e}"))?
                    .code()
                    .unwrap_or(1);
            }
            Ok((code, Vec::new(), Vec::new()))
        }
        "seq" => {
            let (first, last) = match &args[1..] {
                [last] => (1, last.parse::<i64>().map_err(|_| "seq: invalid number")?),
                [first, last] => (
                    first.parse::<i64>().map_err(|_| "seq: invalid number")?,
                    last.parse::<i64>().map_err(|_| "seq: invalid number")?,
                ),
                _ => return Err("seq: expected LAST or FIRST LAST".into()),
            };
            if (last as i128 - first as i128).abs() > 1_000_000 {
                return Err("seq: range exceeds one million lines".into());
            }
            let mut output = Vec::new();
            if first <= last {
                for value in first..=last {
                    output.extend_from_slice(format!("{value}\n").as_bytes());
                }
            } else {
                for value in (last..=first).rev() {
                    output.extend_from_slice(format!("{value}\n").as_bytes());
                }
            }
            ok(output)
        }
        "mktemp" => mktemp(shell, args),
        "tr" => tr(shell, args, io),
        "ls" => {
            let paths: Vec<&str> = args[1..]
                .iter()
                .filter(|arg| !arg.starts_with('-'))
                .map(String::as_str)
                .collect();
            let paths = if paths.is_empty() { vec!["."] } else { paths };
            let mut text = String::new();
            for path in paths {
                let full = shell.path(path);
                if full.is_dir() {
                    let mut entries = fs::read_dir(&full)
                        .map_err(|e| format!("ls: {path}: {e}"))?
                        .map(|entry| entry.map(|e| e.file_name().to_string_lossy().into_owned()))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| e.to_string())?;
                    entries.sort();
                    for entry in entries {
                        text.push_str(&entry);
                        text.push('\n');
                    }
                } else if full.exists() {
                    text.push_str(path);
                    text.push('\n');
                } else {
                    return Err(format!("ls: {path}: no such file or directory"));
                }
            }
            ok(text.into_bytes())
        }
        "mkdir" => {
            let recursive = args.iter().any(|arg| arg == "-p");
            let paths: Vec<_> = args[1..]
                .iter()
                .filter(|arg| !arg.starts_with('-'))
                .collect();
            if paths.is_empty() {
                Err("mkdir: missing operand".into())
            } else {
                for path in paths {
                    let path = shell.path(path);
                    if recursive {
                        fs::create_dir_all(path)
                    } else {
                        fs::create_dir(path)
                    }
                    .map_err(|e| format!("mkdir: {e}"))?;
                }
                ok(Vec::new())
            }
        }
        "touch" => {
            if args.len() == 1 {
                Err("touch: missing operand".into())
            } else {
                for path in &args[1..] {
                    OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(shell.path(path))
                        .map_err(|e| format!("touch: {path}: {e}"))?;
                }
                ok(Vec::new())
            }
        }
        "rm" => remove(shell, args),
        "cp" | "mv" => copy_move(shell, args),
        "test" | "[" => test(shell, args),
        "sleep" => {
            let seconds = args
                .get(1)
                .ok_or("sleep: missing duration")
                .and_then(|v| v.parse::<f64>().map_err(|_| "sleep: invalid duration"));
            match seconds {
                Ok(seconds) if seconds >= 0.0 && seconds.is_finite() => {
                    std::thread::sleep(Duration::from_secs_f64(seconds));
                    ok(Vec::new())
                }
                _ => Err("sleep: invalid duration".into()),
            }
        }
        "env" => {
            let mut vars: Vec<_> = shell.variables.iter().collect();
            vars.sort_by_key(|(key, _)| *key);
            let text = vars
                .into_iter()
                .map(|(key, value)| format!("{key}={value}\n"))
                .collect::<String>();
            ok(text.into_bytes())
        }
        "export" => {
            for arg in &args[1..] {
                if let Some((name, value)) = arg.split_once('=') {
                    shell.variables.set(name, value.into());
                } else if !shell.variables.contains(arg) {
                    shell.variables.set(arg, String::new());
                }
            }
            ok(Vec::new())
        }
        "bash" | "sh" => script(shell, args, io),
        _ => unreachable!(),
    }
}

fn ok(output: Vec<u8>) -> Outcome {
    Ok((0, output, Vec::new()))
}

/// Streams files or standard input to standard output.
fn cat(shell: &mut Shell, args: &[String], io: &Io) -> Outcome {
    let stdin = ["-".to_owned()];
    let operands = if args.len() == 1 {
        &stdin[..]
    } else {
        &args[1..]
    };
    let mut stdout = &io.stdout;
    for operand in operands {
        let copied = if operand == "-" {
            io::copy(&mut &io.stdin, &mut stdout)
        } else {
            let file =
                fs::File::open(shell.path(operand)).map_err(|e| format!("cat: {operand}: {e}"))?;
            io::copy(&mut &file, &mut stdout)
        };
        match copied {
            Ok(_) => {}
            Err(error) if closed(&error) => {
                shell.broken_output = true;
                return Ok((1, Vec::new(), Vec::new()));
            }
            Err(error) => return Err(format!("cat: {error}")),
        }
    }
    if let Err(error) = stdout.flush() {
        if closed(&error) {
            shell.broken_output = true;
            return Ok((1, Vec::new(), Vec::new()));
        }
        return Err(format!("cat: {error}"));
    }
    ok(Vec::new())
}

/// Runs a script file in a fresh shell with its own positional arguments;
/// its state and `exit` do not reach the caller.
fn script(shell: &mut Shell, args: &[String], io: &Io) -> Outcome {
    let Some(path) = args.get(1) else {
        return Err(format!("{}: expected script path", args[0]));
    };
    let text =
        fs::read_to_string(shell.path(path)).map_err(|e| format!("{}: {path}: {e}", args[0]))?;
    let mut child = Shell::with(shell.cwd.clone(), Variables::clone(&shell.variables));
    child.positional = args[2..].to_vec();
    child.arg_zero = path.clone();
    let status = child.run_script(&text, io);
    shell.broken_output |= child.broken_output;
    Ok((status, Vec::new(), Vec::new()))
}

fn test(shell: &Shell, args: &[String]) -> Outcome {
    let words = if args[0] == "[" {
        if args.last().map(String::as_str) != Some("]") {
            return Err("[: missing ]".into());
        }
        &args[1..args.len() - 1]
    } else {
        &args[1..]
    };
    let passed = match words {
        [] => false,
        [value] => !value.is_empty(),
        [flag, value] if flag == "-n" => !value.is_empty(),
        [flag, value] if flag == "-z" => value.is_empty(),
        [flag, path] if flag == "-e" => shell.path(path).exists(),
        [flag, path] if flag == "-f" => shell.path(path).is_file(),
        [flag, path] if flag == "-d" => shell.path(path).is_dir(),
        [left, op, right] if op == "=" || op == "==" => left == right,
        [left, op, right] if op == "!=" => left != right,
        _ => return Err("test: unsupported expression".into()),
    };
    Ok((if passed { 0 } else { 1 }, Vec::new(), Vec::new()))
}
