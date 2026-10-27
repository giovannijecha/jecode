//! Lightweight lexical headings. These anchors are not compiler analysis.
use super::{clip, row};
use crate::{cancel::Cancellation, json::Value};
use std::{fs::File, io::Read, path::Path};

const MAX_SOURCE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SYMBOLS: usize = 500;

pub(super) struct Outline {
    pub entries: Vec<Value>,
    pub truncated: bool,
}

pub(super) fn file(
    absolute: &Path,
    relative: &str,
    cancellation: &Cancellation,
) -> Result<Outline, String> {
    let extension = absolute
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let supported = matches!(extension, "rs" | "md" | "markdown");
    if !supported {
        return Ok(Outline {
            entries: vec![row(
                "landmark",
                relative,
                relative.rsplit('/').next().unwrap_or(relative),
                "File; declaration outline available for Rust and Markdown",
                "filesystem",
                None,
            )],
            truncated: false,
        });
    }
    let mut bytes = Vec::new();
    let file = File::open(absolute).map_err(|error| format!("{relative}: {error}"))?;
    let mut file = file.take(MAX_SOURCE_BYTES + 1);
    let mut buffer = [0u8; 64 * 1024];
    loop {
        if cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("{relative}: {error}"))?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    let truncated = bytes.len() as u64 > MAX_SOURCE_BYTES;
    if truncated {
        bytes.truncate(MAX_SOURCE_BYTES as usize);
    }
    let valid = match std::str::from_utf8(&bytes) {
        Ok(_) => bytes.len(),
        Err(error) if truncated && error.error_len().is_none() => error.valid_up_to(),
        Err(_) => return Err(format!("{relative}: file is not UTF-8 text")),
    };
    let text = std::str::from_utf8(&bytes[..valid]).expect("valid UTF-8 prefix");
    let (mut entries, capped) = if extension == "rs" {
        rust(text, relative, cancellation)?
    } else {
        markdown(text, relative, cancellation)?
    };
    entries.insert(
        0,
        row(
            "landmark",
            relative,
            relative.rsplit('/').next().unwrap_or(relative),
            if extension == "rs" {
                "Rust source file"
            } else {
                "Markdown document"
            },
            "filesystem",
            None,
        ),
    );
    Ok(Outline {
        entries,
        truncated: truncated || capped,
    })
}

fn markdown(
    text: &str,
    path: &str,
    cancellation: &Cancellation,
) -> Result<(Vec<Value>, bool), String> {
    let mut entries = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for (index, raw_line) in text.lines().enumerate() {
        if cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        let line = raw_line.trim_start_matches(' ');
        if raw_line.len() - line.len() > 3 || line.starts_with('\t') {
            continue;
        }
        let marker = line.chars().next().unwrap_or(' ');
        let marker_count = line
            .chars()
            .take_while(|character| *character == marker)
            .count();
        if let Some((open, count)) = fence {
            if marker == open && marker_count >= count && line[marker_count..].trim().is_empty() {
                fence = None;
            }
            continue;
        }
        if (marker == '`' || marker == '~') && marker_count >= 3 {
            fence = Some((marker, marker_count));
            continue;
        }
        if marker == '#' && (1..=6).contains(&marker_count) {
            let heading = line[marker_count..].strip_prefix(' ').unwrap_or("").trim();
            if !heading.is_empty() {
                let heading = clip(heading, 160);
                entries.push(row(
                    "symbol",
                    path,
                    &heading,
                    "Markdown heading",
                    "source",
                    Some((index + 1, clip(raw_line, 180))),
                ));
                if entries.len() >= MAX_SYMBOLS {
                    return Ok((entries, true));
                }
            }
        }
    }
    Ok((entries, false))
}

fn rust(text: &str, path: &str, cancellation: &Cancellation) -> Result<(Vec<Value>, bool), String> {
    let masked = mask_rust(text, cancellation)?;
    let mut entries = Vec::new();
    for (index, line) in masked.lines().enumerate() {
        if cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        let line = line.trim();
        let Some(role) = declaration_role(line) else {
            continue;
        };
        let title = line
            .split(['{', ';'])
            .next()
            .unwrap_or(line)
            .split('=')
            .next()
            .unwrap_or(line)
            .trim();
        if title.is_empty() {
            continue;
        }
        entries.push(row(
            "symbol",
            path,
            &clip(title, 160),
            role,
            "source",
            Some((index + 1, clip(title, 180))),
        ));
        if entries.len() >= MAX_SYMBOLS {
            return Ok((entries, true));
        }
    }
    Ok((entries, false))
}

fn declaration_role(line: &str) -> Option<&'static str> {
    const KINDS: [(&str, &str); 9] = [
        ("fn", "Rust function declaration (lexical)"),
        ("struct", "Rust struct declaration (lexical)"),
        ("enum", "Rust enum declaration (lexical)"),
        ("trait", "Rust trait declaration (lexical)"),
        ("impl", "Rust implementation declaration (lexical)"),
        ("mod", "Rust module declaration (lexical)"),
        ("type", "Rust type declaration (lexical)"),
        ("const", "Rust constant declaration (lexical)"),
        ("static", "Rust static declaration (lexical)"),
    ];
    for (word, role) in KINDS {
        for (position, _) in line.match_indices(word) {
            let before = &line[..position];
            let after = &line[position + word.len()..];
            if (after.is_empty()
                || after.starts_with(|character: char| {
                    !character.is_alphanumeric() && character != '_'
                }))
                && prefix_is_modifiers(before)
            {
                return Some(role);
            }
        }
    }
    None
}

fn prefix_is_modifiers(prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    let mut words = prefix.split_whitespace();
    words.all(|word| {
        matches!(
            word,
            "pub" | "async" | "unsafe" | "default" | "extern" | "auto" | "const"
        ) || word.starts_with("pub(") && word.ends_with(')')
    })
}

fn mask_rust(text: &str, cancellation: &Cancellation) -> Result<String, String> {
    let bytes = text.as_bytes();
    let mut output = bytes.to_vec();
    let mut index = 0;
    let mut block_depth = 0usize;
    let mut line_comment = false;
    let mut quoted: Option<(u8, usize, bool)> = None;
    while index < bytes.len() {
        if index % 16384 == 0 && cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        let current = bytes[index];
        if current == b'\n' {
            line_comment = false;
            index += 1;
            continue;
        }
        if line_comment {
            output[index] = b' ';
            index += 1;
            continue;
        }
        if block_depth > 0 {
            if bytes.get(index..index + 2) == Some(b"/*") {
                output[index..index + 2].fill(b' ');
                block_depth += 1;
                index += 2;
            } else if bytes.get(index..index + 2) == Some(b"*/") {
                output[index..index + 2].fill(b' ');
                block_depth -= 1;
                index += 2;
            } else {
                output[index] = b' ';
                index += 1;
            }
            continue;
        }
        if let Some((quote, hashes, raw)) = quoted {
            output[index] = b' ';
            if !raw && current == b'\\' {
                if let Some(next) = output.get_mut(index + 1) {
                    *next = b' ';
                }
                index += 2;
            } else if current == quote
                && (hashes == 0
                    || bytes
                        .get(index + 1..index + 1 + hashes)
                        .is_some_and(|suffix| suffix.iter().all(|byte| *byte == b'#')))
            {
                output[index..index + 1 + hashes].fill(b' ');
                quoted = None;
                index += 1 + hashes;
            } else {
                index += 1;
            }
            continue;
        }
        if bytes.get(index..index + 2) == Some(b"//") {
            output[index..index + 2].fill(b' ');
            line_comment = true;
            index += 2;
        } else if bytes.get(index..index + 2) == Some(b"/*") {
            output[index..index + 2].fill(b' ');
            block_depth = 1;
            index += 2;
        } else if current == b'r' {
            let mut end = index + 1;
            while bytes.get(end) == Some(&b'#') {
                end += 1;
            }
            if bytes.get(end) == Some(&b'"') {
                output[index..=end].fill(b' ');
                quoted = Some((b'"', end - index - 1, true));
                index = end + 1;
            } else {
                index += 1;
            }
        } else if current == b'"' {
            output[index] = b' ';
            quoted = Some((b'"', 0, false));
            index += 1;
        } else if current == b'\'' && char_literal(bytes, index) {
            output[index] = b' ';
            quoted = Some((b'\'', 0, false));
            index += 1;
        } else {
            index += 1;
        }
    }
    String::from_utf8(output).map_err(|_| "Invalid UTF-8 while parsing Rust".into())
}

fn char_literal(bytes: &[u8], index: usize) -> bool {
    let mut end = index + 1;
    if bytes.get(end) == Some(&b'\\') {
        end += 2;
    } else {
        end += 1;
    }
    bytes.get(end) == Some(&b'\'')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_ignores_comments_and_strings() {
        let source = "// fn fake() {}\nlet s = r#\"fn fake2() {}\"#;\n/* fn fake3() {} */\npub fn real() {}\n";
        let (entries, truncated) = rust(source, "src/lib.rs", &Cancellation::default()).unwrap();
        assert!(!truncated);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].get("title").and_then(Value::as_str),
            Some("pub fn real()")
        );
        assert_eq!(
            entries[0].get("sources").and_then(Value::as_array).unwrap()[0]
                .get("line")
                .and_then(Value::as_usize),
            Some(4)
        );
    }

    #[test]
    fn markdown_skips_fenced_headings() {
        let source = "# Real\n```md\n# Fake\n```\n## Next\n";
        let (entries, _) = markdown(source, "README.md", &Cancellation::default()).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn markdown_requires_a_real_closing_fence_and_skips_indented_code() {
        let source = "```md\n```text\n# Still fenced\n```\n    # Indented code\n   ## Real\n";
        let (entries, _) = markdown(source, "README.md", &Cancellation::default()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].get("title").and_then(Value::as_str),
            Some("Real")
        );
        assert_eq!(
            entries[0].get("sources").and_then(Value::as_array).unwrap()[0]
                .get("line")
                .and_then(Value::as_usize),
            Some(6)
        );
    }

    #[test]
    fn rust_raw_string_backslash_does_not_hide_following_declaration() {
        let source = "let text = r\"backslash\\\";\nfn visible() {}\n";
        let (entries, _) = rust(source, "src/lib.rs", &Cancellation::default()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].get("title").and_then(Value::as_str),
            Some("fn visible()")
        );
    }

    #[test]
    fn rust_nested_comments_strings_chars_and_lifetimes_keep_lines() {
        let source = "/* outer /* fn hidden() {} */ still hidden */\nlet quote = '\\'';\nlet text = \"fn hidden2() {}\";\nfn visible<'a>(value: &'a str) {}\n";
        let (entries, _) = rust(source, "src/lib.rs", &Cancellation::default()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].get("sources").and_then(Value::as_array).unwrap()[0]
                .get("line")
                .and_then(Value::as_usize),
            Some(4)
        );
    }
}
