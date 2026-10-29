use super::Shell;
#[path = "builtins/format.rs"]
mod format;
use format::printf;
#[path = "builtins/text.rs"]
mod text;
use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use text::tr;

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

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

pub(super) fn run(
    shell: &mut Shell,
    args: &[String],
    input: Option<&[u8]>,
    stream_direct: bool,
) -> Option<Outcome> {
    if !is_builtin(&args[0]) || args[0] == "env" && args.len() > 1 {
        return None;
    }
    Some(run_builtin(shell, args, input, stream_direct))
}

fn run_builtin(
    shell: &mut Shell,
    args: &[String],
    input: Option<&[u8]>,
    stream_direct: bool,
) -> Outcome {
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
                .or_else(|| shell.variables.get("HOME").map(String::as_str));
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
        "cat" => {
            if args.len() == 1 || args[1..].iter().any(|arg| arg == "-") {
                if stream_direct && input.is_none() {
                    io::copy(&mut io::stdin(), &mut io::stdout())
                        .map_err(|e| format!("cat: {e}"))?;
                    ok(Vec::new())
                } else if let Some(input) = input {
                    ok(input.to_vec())
                } else {
                    let mut bytes = Vec::new();
                    io::stdin()
                        .read_to_end(&mut bytes)
                        .map_err(|e| format!("cat: {e}"))?;
                    ok(bytes)
                }
            } else {
                let mut bytes = Vec::new();
                for arg in &args[1..] {
                    bytes
                        .extend(fs::read(shell.path(arg)).map_err(|e| format!("cat: {arg}: {e}"))?);
                }
                ok(bytes)
            }
        }
        "wc" => {
            if args.get(1).map(String::as_str) != Some("-c") || args.len() != 2 {
                return Err("wc: only -c from stdin is supported".into());
            }
            let count = if let Some(input) = input {
                input.len() as u64
            } else {
                io::copy(&mut io::stdin(), &mut io::sink()).map_err(|e| format!("wc: {e}"))?
            };
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
        "tr" => tr(args, input, stream_direct),
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
                    shell.variables.insert(name.into(), value.into());
                } else if !shell.variables.contains_key(arg) {
                    shell.variables.insert(arg.into(), String::new());
                }
            }
            ok(Vec::new())
        }
        "bash" | "sh" => {
            let Some(path) = args.get(1) else {
                return Err("bash: expected script path".into());
            };
            match fs::read_to_string(shell.path(path)) {
                Ok(script) => Ok((shell.run(&script), Vec::new(), Vec::new())),
                Err(e) => Err(format!("bash: {path}: {e}")),
            }
        }
        _ => unreachable!(),
    }
}

fn ok(output: Vec<u8>) -> Outcome {
    Ok((0, output, Vec::new()))
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

fn remove(shell: &Shell, args: &[String]) -> Outcome {
    let recursive = args
        .iter()
        .any(|arg| arg == "-r" || arg == "-R" || arg == "-rf" || arg == "-fr");
    let force = args
        .iter()
        .any(|arg| arg == "-f" || arg == "-rf" || arg == "-fr");
    let paths: Vec<_> = args[1..]
        .iter()
        .filter(|arg| !arg.starts_with('-'))
        .collect();
    if paths.is_empty() {
        return Err("rm: missing operand".into());
    }
    for path in paths {
        let full = shell.path(path);
        let result = if full.is_dir() {
            if recursive {
                fs::remove_dir_all(&full)
            } else {
                fs::remove_dir(&full)
            }
        } else {
            fs::remove_file(&full)
        };
        if let Err(error) = result
            && (!force || full.exists())
        {
            return Err(format!("rm: {path}: {error}"));
        }
    }
    ok(Vec::new())
}

fn copy_move(shell: &Shell, args: &[String]) -> Outcome {
    let recursive = args.iter().any(|arg| arg == "-r" || arg == "-R");
    let operands: Vec<_> = args[1..]
        .iter()
        .filter(|arg| !arg.starts_with('-'))
        .collect();
    if operands.len() != 2 {
        return Err(format!("{}: expected source and destination", args[0]));
    }
    let source = shell.path(operands[0]);
    let mut destination = shell.path(operands[1]);
    if destination.is_dir() {
        destination.push(source.file_name().ok_or("invalid source")?);
    }
    if args[0] == "mv" {
        fs::rename(&source, &destination).map_err(|e| format!("mv: {e}"))?;
    } else if source.is_dir() {
        if !recursive {
            return Err("cp: directory requires -r".into());
        }
        copy_tree(&source, &destination)?;
    } else {
        fs::copy(&source, &destination).map_err(|e| format!("cp: {e}"))?;
    }
    ok(Vec::new())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let target = destination.join(entry.file_name());
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn mktemp(shell: &Shell, args: &[String]) -> Outcome {
    let directory = args.iter().any(|arg| arg == "-d");
    let templates: Vec<_> = args[1..].iter().filter(|arg| *arg != "-d").collect();
    if templates.len() > 1 {
        return Err("mktemp: expected one template".into());
    }
    let template = if let Some(template) = templates.first() {
        (*template).clone()
    } else {
        let base = shell
            .variables
            .get("TMPDIR")
            .map(String::as_str)
            .unwrap_or(".");
        format!("{base}/jecode.XXXXXXXXXX")
    };
    let x_count = template.chars().rev().take_while(|ch| *ch == 'X').count();
    if !(6..=64).contains(&x_count) {
        return Err("mktemp: template must end in 6 to 64 X characters".into());
    }
    let prefix = &template[..template.len() - x_count];
    for _ in 0..100 {
        let nonce = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let suffix = format!(
            "{:0width$x}",
            (std::process::id() as u64) << 32 | nonce,
            width = x_count
        );
        let path = shell.path(&format!("{prefix}{suffix}"));
        let result = if directory {
            fs::create_dir(&path)
        } else {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map(|_| ())
        };
        match result {
            Ok(()) => return ok(format!("{}\n", path.display()).into_bytes()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("mktemp: {}: {error}", path.display())),
        }
    }
    Err("mktemp: could not allocate a unique name".into())
}
