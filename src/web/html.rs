use super::url::Url;
use std::collections::HashSet;

pub(super) struct Document {
    pub title: Option<String>,
    pub text: String,
    pub links: Vec<String>,
}

pub(super) fn extract(source: &str, base: &Url) -> Document {
    let mut text = String::new();
    let mut title = String::new();
    let mut in_title = false;
    let mut skip = None::<String>;
    let mut code = false;
    let mut links = Vec::new();
    let mut seen = HashSet::new();
    let mut position = 0;
    let bytes = source.as_bytes();
    while position < source.len() {
        let Some(open) = source[position..].find('<').map(|offset| position + offset) else {
            append_text(
                &source[position..],
                &mut text,
                &mut title,
                in_title,
                skip.is_some(),
                code,
            );
            break;
        };
        append_text(
            &source[position..open],
            &mut text,
            &mut title,
            in_title,
            skip.is_some(),
            code,
        );
        let Some(close) = tag_end(bytes, open + 1) else {
            append_text(
                &source[open..],
                &mut text,
                &mut title,
                in_title,
                skip.is_some(),
                code,
            );
            break;
        };
        let raw = source[open + 1..close].trim();
        position = close + 1;
        if raw.starts_with('!') || raw.starts_with('?') {
            continue;
        }
        let closing = raw.starts_with('/');
        let content = raw.trim_start_matches('/').trim_start();
        let end = content
            .find(|ch: char| ch.is_whitespace() || ch == '/')
            .unwrap_or(content.len());
        let name = content[..end].to_ascii_lowercase();
        if closing {
            if skip.as_deref() == Some(name.as_str()) {
                skip = None;
            }
            if name == "title" {
                in_title = false;
            }
            if name == "pre" && code {
                text.push_str("\n```\n");
                code = false;
            }
            if matches!(
                name.as_str(),
                "p" | "div"
                    | "section"
                    | "article"
                    | "li"
                    | "tr"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
            ) {
                line_break(&mut text);
            }
            continue;
        }
        if skip.is_some() {
            continue;
        }
        if matches!(name.as_str(), "script" | "style" | "svg") {
            skip = Some(name);
            continue;
        }
        if name == "title" {
            in_title = true;
            continue;
        }
        if let Some(level) = name
            .strip_prefix('h')
            .and_then(|value| value.parse::<usize>().ok())
        {
            if (1..=6).contains(&level) {
                line_break(&mut text);
                text.push_str(&"#".repeat(level));
                text.push(' ');
            }
        } else if name == "pre" {
            line_break(&mut text);
            text.push_str("```\n");
            code = true;
        } else if name == "br" {
            line_break(&mut text);
        } else if name == "li" {
            line_break(&mut text);
            text.push_str("- ");
        } else if matches!(name.as_str(), "p" | "div" | "section" | "article" | "tr") {
            line_break(&mut text);
        } else if matches!(name.as_str(), "td" | "th") && !text.ends_with('\n') && !text.is_empty()
        {
            text.push_str(" | ");
        }
        if name == "a"
            && let Some(href) = attribute(&content[end..], "href")
            && let Some(url) = base.resolve(&decode(&href))
            && seen.insert(url.value.clone())
        {
            links.push(url.value);
        }
    }
    Document {
        title: (!title.trim().is_empty()).then(|| title.trim().to_string()),
        text: text.trim().to_string(),
        links,
    }
}

fn append_text(
    fragment: &str,
    text: &mut String,
    title: &mut String,
    in_title: bool,
    skipped: bool,
    code: bool,
) {
    if skipped {
        return;
    }
    let decoded = decode(fragment);
    if in_title {
        title.push_str(&decoded);
        return;
    }
    if code {
        text.push_str(&decoded);
        return;
    }
    for ch in decoded.chars() {
        if ch.is_whitespace() {
            if !text.ends_with([' ', '\n']) && !text.is_empty() {
                text.push(' ');
            }
        } else {
            text.push(ch);
        }
    }
}

fn line_break(text: &mut String) {
    while text.ends_with(' ') {
        text.pop();
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
}

fn tag_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut quote = 0;
    for (index, &byte) in bytes.iter().enumerate().skip(start) {
        if quote != 0 {
            if byte == quote {
                quote = 0;
            }
        } else if byte == b'\'' || byte == b'"' {
            quote = byte;
        } else if byte == b'>' {
            return Some(index);
        }
    }
    None
}

fn attribute(input: &str, wanted: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        while index < bytes.len() && (bytes[index].is_ascii_whitespace() || bytes[index] == b'/') {
            index += 1;
        }
        let start = index;
        while index < bytes.len()
            && !bytes[index].is_ascii_whitespace()
            && !matches!(bytes[index], b'=' | b'/' | b'>')
        {
            index += 1;
        }
        if index == start {
            index += 1;
            continue;
        }
        let name = &input[start..index];
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if bytes.get(index) != Some(&b'=') {
            continue;
        }
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let value = if let Some(&quote @ (b'\'' | b'"')) = bytes.get(index) {
            index += 1;
            let start = index;
            while index < bytes.len() && bytes[index] != quote {
                index += 1;
            }
            let value = &input[start..index];
            if index < bytes.len() {
                index += 1;
            }
            value
        } else {
            let start = index;
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() && bytes[index] != b'>'
            {
                index += 1;
            }
            &input[start..index]
        };
        if name.eq_ignore_ascii_case(wanted) {
            return Some(value.to_string());
        }
    }
    None
}

fn decode(input: &str) -> String {
    let mut output = String::new();
    let mut rest = input;
    while let Some(index) = rest.find('&') {
        output.push_str(&rest[..index]);
        rest = &rest[index..];
        let Some(end) = rest.find(';').filter(|&end| end <= 12) else {
            output.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            value if value.starts_with("#x") => u32::from_str_radix(&value[2..], 16)
                .ok()
                .and_then(char::from_u32),
            value if value.starts_with('#') => value[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        if let Some(ch) = decoded {
            output.push(ch);
        } else {
            output.push_str(&rest[..=end]);
        }
        rest = &rest[end + 1..];
    }
    output.push_str(rest);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_links_title_and_structure() {
        let base = Url::parse("http://example.com/a/page").unwrap();
        let document = extract(
            "<title>A &amp; B</title><script>hidden</script><h2>Heading</h2><p>Hello <a href='../b?x=1&amp;y=2'>there</a></p><pre>x\ny</pre><table><tr><td>one</td><td>two</td></tr></table>",
            &base,
        );
        assert_eq!(document.title.as_deref(), Some("A & B"));
        assert_eq!(document.links, ["http://example.com/b?x=1&y=2"]);
        assert!(document.text.contains("## Heading"));
        assert!(document.text.contains("```"));
        assert!(document.text.contains("one | two"));
        assert!(!document.text.contains("hidden"));
    }
}
