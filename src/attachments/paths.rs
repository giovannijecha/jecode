//! Recognizes a paste that consists only of dropped file paths.
//!
//! Terminals deliver a file drop as pasted text. Windows Terminal separates
//! paths with spaces and double-quotes paths containing spaces; WSL profiles
//! single-quote every translated path with `'\''` for an embedded quote; Unix
//! terminals use single quotes, backslash escapes or `file://` URIs. Quotes
//! from PowerShell use doubled apostrophes inside single-quoted paths.
//! and escapes are only removed, never evaluated: no expansion, globbing or
//! command execution. The paste counts as a drop only when every path is an
//! existing regular file, so prose and directories stay text.

use std::path::PathBuf;

const MAX_TEXT: usize = 64 * 1024;
const MAX_PATHS: usize = 64;

pub fn dropped(text: &str) -> Option<Vec<PathBuf>> {
    dropped_with(text, &native)
}

/// `resolve` maps one path token to a local regular file.
pub fn dropped_with(text: &str, resolve: &dyn Fn(&str) -> Option<PathBuf>) -> Option<Vec<PathBuf>> {
    let text = text.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    if text.is_empty() || text.len() > MAX_TEXT {
        return None;
    }
    let all = |words: Vec<String>| {
        (!words.is_empty() && words.len() <= MAX_PATHS)
            .then(|| {
                words
                    .iter()
                    .map(|word| resolve(word))
                    .collect::<Option<Vec<_>>>()
            })
            .flatten()
    };
    for escapes in [false, true] {
        if let Some(paths) = split(text, escapes).and_then(all) {
            return Some(paths);
        }
    }
    // One unquoted path that contains spaces.
    resolve(text).map(|path| vec![path])
}

/// One path argument: absolute, a `file:` URI, or relative to `directory`.
/// Under WSL a Windows path is translated to its mount.
pub fn argument(word: &str, directory: &std::path::Path) -> PathBuf {
    local(word).map_or_else(|| directory.join(word), PathBuf::from)
}

/// Paths typed after `/attach`: quoted or bare, absolute or relative to
/// `directory`. Backslashes stay literal so Windows paths need no escaping.
pub fn typed(text: &str, directory: &std::path::Path) -> Result<Vec<PathBuf>, String> {
    let resolve = |word: &str| argument(word, directory);
    let text = text.trim();
    let words = split(text, false).ok_or("A quote in the path list is not closed")?;
    if words.is_empty() {
        return Err("Usage: /attach PATH...".into());
    }
    let paths: Vec<_> = words.iter().map(|word| resolve(word)).collect();
    if paths.len() > 1 && !paths.iter().all(|path| path.is_file()) && resolve(text).is_file() {
        return Ok(vec![resolve(text)]);
    }
    if paths.len() > MAX_PATHS {
        return Err(format!("Attach at most {MAX_PATHS} files at once"));
    }
    Ok(paths)
}

/// Quoted path or attachment-reference arguments, without shell evaluation.
pub fn words(text: &str) -> Result<Vec<String>, String> {
    let words = split(text.trim(), false).ok_or("A quote in the path list is not closed")?;
    if words.is_empty() {
        return Err("Usage: /attach PATH...".into());
    }
    if words.len() > MAX_PATHS {
        return Err(format!("Attach at most {MAX_PATHS} files at once"));
    }
    Ok(words)
}

/// Shell-like word splitting limited to quote removal.
fn split(text: &str, escapes: bool) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            '\'' => {
                started = true;
                loop {
                    match chars.next()? {
                        '\'' if chars.peek() == Some(&'\'') => {
                            chars.next();
                            word.push('\'');
                        }
                        '\'' => break,
                        c => word.push(c),
                    }
                }
            }
            '"' => {
                started = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' if escapes => match chars.next()? {
                            c @ ('"' | '\\' | '$' | '`') => word.push(c),
                            c => {
                                word.push('\\');
                                word.push(c);
                            }
                        },
                        c => word.push(c),
                    }
                }
            }
            '\\' if escapes => {
                started = true;
                word.push(chars.next()?);
            }
            c => {
                started = true;
                word.push(c);
            }
        }
    }
    if started {
        words.push(word);
    }
    Some(words)
}

fn native(word: &str) -> Option<PathBuf> {
    let path = PathBuf::from(local(word)?);
    std::fs::metadata(&path)
        .is_ok_and(|metadata| metadata.is_file())
        .then_some(path)
}

/// The local form of an absolute path or `file:` URI. Relative words are
/// rejected: a drop always names absolute paths, typed words rarely do.
fn local(word: &str) -> Option<String> {
    let word = match word.strip_prefix("file://") {
        Some(rest) => {
            let rest = rest.strip_prefix("localhost").unwrap_or(rest);
            let decoded = percent_decode(rest)?;
            // file:///C:/x names the Windows path C:/x.
            match decoded.strip_prefix('/') {
                Some(windows) if drive(windows) => windows.to_owned(),
                _ => decoded,
            }
        }
        None => word.to_owned(),
    };
    if drive(&word) || word.starts_with(r"\\") {
        return windows_path(word);
    }
    word.starts_with('/').then_some(word)
}

fn drive(word: &str) -> bool {
    let bytes = word.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

#[cfg(windows)]
fn windows_path(word: String) -> Option<String> {
    Some(word)
}

/// Under WSL, Windows paths go through `wslpath`, which follows the live
/// mount table instead of assuming `/mnt/<drive>`.
#[cfg(not(windows))]
fn windows_path(word: String) -> Option<String> {
    if !wsl() {
        return None;
    }
    if let Some(path) = std::env::var("WSL_DISTRO_NAME")
        .ok()
        .and_then(|distro| distro_path(&word, &distro))
    {
        return Some(path);
    }
    let output = std::process::Command::new("wslpath")
        .args(["-u", "-a", "--", &word])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let path = String::from_utf8(output.stdout).ok()?;
    let path = path.strip_suffix('\n').unwrap_or(&path);
    (output.status.success() && path.starts_with('/')).then(|| path.to_owned())
}

/// `wslpath` treats UNC paths into the running distribution as network
/// shares; those files are reachable at their Linux path directly.
#[cfg_attr(windows, allow(dead_code))]
fn distro_path(word: &str, distro: &str) -> Option<String> {
    let rest = word
        .strip_prefix(r"\\wsl.localhost\")
        .or_else(|| word.strip_prefix(r"\\wsl$\"))?;
    let (name, path) = rest.split_once('\\').unwrap_or((rest, ""));
    name.eq_ignore_ascii_case(distro)
        .then(|| format!("/{}", path.replace('\\', "/")))
}

#[cfg(not(windows))]
pub fn wsl() -> bool {
    std::env::var_os("WSL_DISTRO_NAME").is_some()
        || std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .is_ok_and(|release| release.to_ascii_lowercase().contains("microsoft"))
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hex = std::str::from_utf8(bytes.get(at + 1..at + 3)?).ok()?;
            output.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            output.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(output).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Directory;
    use std::fs;

    fn resolve_in(root: &std::path::Path) -> impl Fn(&str) -> Option<PathBuf> + '_ {
        move |word| {
            // Stand-in for wslpath in tests on every platform.
            let converted = word
                .strip_prefix(r"C:\fixture\")
                .map(|rest| format!("/fixture/{}", rest.replace('\\', "/")));
            let path = PathBuf::from(converted.or_else(|| local(word))?);
            let relative = path.strip_prefix("/fixture").ok()?;
            fs::metadata(root.join(relative))
                .is_ok_and(|metadata| metadata.is_file())
                .then(|| root.join(relative))
        }
    }

    #[test]
    fn recognizes_complete_lists_of_existing_files() {
        let directory = Directory::new();
        for name in ["a.png", "with space.pdf", "ünï 名.txt", "it's.log"] {
            fs::write(directory.path().join(name), b"x").unwrap();
        }
        fs::create_dir(directory.path().join("folder")).unwrap();
        let resolve = resolve_in(directory.path());
        let names = |text: &str| {
            dropped_with(text, &resolve).map(|paths| {
                paths
                    .iter()
                    .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
            })
        };
        // Windows Terminal: quotes only around paths with spaces.
        assert_eq!(
            names("/fixture/a.png \"/fixture/with space.pdf\" /fixture/ünï 名.txt"),
            None
        );
        assert_eq!(
            names("/fixture/a.png \"/fixture/with space.pdf\" \"/fixture/ünï 名.txt\""),
            Some(vec![
                "a.png".into(),
                "with space.pdf".into(),
                "ünï 名.txt".into()
            ])
        );
        // WSL profile style with an escaped single quote.
        assert_eq!(
            names("'/fixture/it'\\''s.log' '/fixture/a.png'"),
            Some(vec!["it's.log".into(), "a.png".into()])
        );
        // Backslash escapes and URIs from Unix terminals.
        assert_eq!(
            names("/fixture/with\\ space.pdf\n"),
            Some(vec!["with space.pdf".into()])
        );
        assert_eq!(
            names("file:///fixture/with%20space.pdf\r\nfile:///fixture/a.png"),
            Some(vec!["with space.pdf".into(), "a.png".into()])
        );
        // A single unquoted path with spaces.
        assert_eq!(
            names("/fixture/with space.pdf"),
            Some(vec!["with space.pdf".into()])
        );
        // Converted Windows-origin paths.
        assert_eq!(
            names(r#""C:\fixture\with space.pdf" C:\fixture\a.png"#),
            Some(vec!["with space.pdf".into(), "a.png".into()])
        );
        // Prose, partial lists, directories, missing files and relative words stay text.
        for text in [
            "look at /fixture/a.png please",
            "/fixture/a.png /fixture/missing.png",
            "/fixture/folder",
            "a.png",
            "'/fixture/a.png",
            "",
            "   ",
        ] {
            assert_eq!(names(text), None, "{text:?}");
        }
    }

    #[test]
    fn native_resolution_requires_an_existing_absolute_file() {
        let directory = Directory::new();
        let file = directory.path().join("report final.pdf");
        fs::write(&file, b"%PDF-").unwrap();
        let quoted = format!("\"{}\"", file.display());
        assert_eq!(dropped(&quoted), Some(vec![file.clone()]));
        assert_eq!(dropped("report final.pdf"), None);
        assert_eq!(dropped(&directory.path().display().to_string()), None);
    }

    #[test]
    fn unc_paths_into_the_running_distribution_map_directly() {
        assert_eq!(
            distro_path(r"\\wsl.localhost\Ubuntu\home\me\a b.txt", "Ubuntu").as_deref(),
            Some("/home/me/a b.txt")
        );
        assert_eq!(
            distro_path(r"\\wsl$\ubuntu\etc\hosts", "Ubuntu").as_deref(),
            Some("/etc/hosts")
        );
        assert_eq!(distro_path(r"\\wsl.localhost\Debian\x", "Ubuntu"), None);
        assert_eq!(distro_path(r"\\server\share\x", "Ubuntu"), None);
    }

    #[test]
    fn powershell_literals_preserve_apostrophes_and_shell_characters() {
        let directory = Directory::new();
        let file = directory.path().join("report ' $x ` $(1+2) β.bin");
        fs::write(&file, b"exact bytes").unwrap();
        let quoted = format!("'{}'", file.display().to_string().replace('\'', "''"));
        assert_eq!(dropped(&quoted), Some(vec![file.clone()]));
        assert_eq!(typed(&quoted, directory.path()).unwrap(), vec![file]);
    }

    #[test]
    fn typed_paths_resolve_relative_names_and_unquoted_spaces() {
        let directory = Directory::new();
        let root = directory.path();
        fs::write(root.join("a.txt"), "a").unwrap();
        fs::write(root.join("my file.txt"), "b").unwrap();
        assert_eq!(
            typed("a.txt \"my file.txt\"", root).unwrap(),
            [root.join("a.txt"), root.join("my file.txt")]
        );
        assert_eq!(
            typed(" my file.txt ", root).unwrap(),
            [root.join("my file.txt")]
        );
        // A missing file reaches the import, which names it in its error.
        assert_eq!(
            typed("missing.txt", root).unwrap(),
            [root.join("missing.txt")]
        );
        assert!(typed("  ", root).is_err());
        assert!(typed("\"open", root).is_err());
        let absolute = root.join("a.txt");
        let elsewhere = Directory::new();
        assert_eq!(
            typed(&format!("\"{}\"", absolute.display()), elsewhere.path()).unwrap(),
            [absolute]
        );
    }
}
