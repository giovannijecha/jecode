//! Copyable parts of an assistant response, extracted from its original text.

#[derive(Debug, PartialEq, Eq)]
pub struct Target {
    pub name: String,
    pub text: String,
}

struct Line<'a> {
    body: &'a str,
    ending: &'a str,
}

struct Fence<'a> {
    marker: u8,
    count: usize,
    language: Option<&'a str>,
}

/// Returns the full response first, followed by copyable blocks in source order.
pub fn extract(source: &str) -> Vec<Target> {
    let lines: Vec<_> = source
        .split_inclusive('\n')
        .map(|line| {
            let (body, ending) = match line.strip_suffix("\r\n") {
                Some(body) => (body, "\r\n"),
                None => match line.strip_suffix('\n') {
                    Some(body) => (body, "\n"),
                    None => (line, ""),
                },
            };
            Line { body, ending }
        })
        .collect();
    let mut targets = vec![Target {
        name: "Whole response".into(),
        text: source.into(),
    }];
    let (mut line, mut code_count, mut quote_count) = (0, 0, 0);

    while line < lines.len() {
        if quote_prefix(lines[line].body).is_some() {
            let start = line;
            while line < lines.len() && quote_prefix(lines[line].body).is_some() {
                line += 1;
            }
            quote_count += 1;
            let mut text = String::new();
            for source_line in &lines[start..line] {
                text.push_str(quote_prefix(source_line.body).expect("quoted range"));
                text.push_str(source_line.ending);
            }
            if !text.is_empty() {
                targets.push(Target {
                    name: format!("Blockquote {quote_count}"),
                    text,
                });
            }
            scan_quoted_fences(&lines, start, line, &mut code_count, &mut targets);
        } else if let Some(fence) = opening_fence(lines[line].body) {
            code_count += 1;
            let (next, text) = collect_fence(&lines, line, lines.len(), 0, &fence);
            if !text.is_empty() {
                targets.push(Target {
                    name: code_name(code_count, fence.language),
                    text,
                });
            }
            line = next;
        } else {
            line += 1;
        }
    }
    targets
}

fn scan_quoted_fences(
    lines: &[Line<'_>],
    start: usize,
    end: usize,
    code_count: &mut usize,
    targets: &mut Vec<Target>,
) {
    let mut line = start;
    while line < end {
        let (depth, body) = quote_depth(lines[line].body);
        if let Some(fence) = opening_fence(body) {
            *code_count += 1;
            let (next, text) = collect_fence(lines, line, end, depth, &fence);
            if !text.is_empty() {
                targets.push(Target {
                    name: code_name(*code_count, fence.language),
                    text,
                });
            }
            line = next;
        } else {
            line += 1;
        }
    }
}

fn collect_fence(
    lines: &[Line<'_>],
    opening: usize,
    end: usize,
    depth: usize,
    fence: &Fence<'_>,
) -> (usize, String) {
    let mut text = String::new();
    let mut line = opening + 1;
    while line < end {
        let Some(body) = strip_quotes(lines[line].body, depth) else {
            break;
        };
        if closing_fence(body, fence) {
            line += 1;
            break;
        }
        text.push_str(body);
        text.push_str(lines[line].ending);
        line += 1;
    }
    (line, text)
}

fn code_name(count: usize, language: Option<&str>) -> String {
    match language {
        Some(language) => format!("Code block {count} · {language}"),
        None => format!("Code block {count}"),
    }
}

fn opening_fence(body: &str) -> Option<Fence<'_>> {
    let body = body.trim_start_matches([' ', '\t']);
    let marker = *body.as_bytes().first()?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }
    let count = body.bytes().take_while(|&byte| byte == marker).count();
    if count < 3 {
        return None;
    }
    let info = body[count..].trim();
    if marker == b'`' && info.contains('`') {
        return None;
    }
    Some(Fence {
        marker,
        count,
        language: info.split_whitespace().next(),
    })
}

fn closing_fence(body: &str, fence: &Fence<'_>) -> bool {
    let body = body.trim_start_matches([' ', '\t']);
    let count = body
        .bytes()
        .take_while(|&byte| byte == fence.marker)
        .count();
    count >= fence.count
        && body[count..]
            .bytes()
            .all(|byte| matches!(byte, b' ' | b'\t'))
}

fn quote_prefix(body: &str) -> Option<&str> {
    let body = body.trim_start_matches([' ', '\t']);
    let body = body.strip_prefix('>')?;
    Some(body.strip_prefix(' ').unwrap_or(body))
}

fn quote_depth(mut body: &str) -> (usize, &str) {
    let mut depth = 0;
    while let Some(rest) = quote_prefix(body) {
        body = rest;
        depth += 1;
    }
    (depth, body)
}

fn strip_quotes(mut body: &str, depth: usize) -> Option<&str> {
    for _ in 0..depth {
        body = quote_prefix(body)?;
    }
    Some(body)
}

#[cfg(test)]
#[path = "targets/tests.rs"]
mod tests;
