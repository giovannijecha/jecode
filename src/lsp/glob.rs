//! Compiled globs for LSP watched-file registrations.
use std::path::PathBuf;

const MAX_PATTERN_CHARS: usize = 4096;
const MAX_ALTERNATIVES: usize = 128;
const MAX_BRACE_GROUPS: usize = 32;

#[derive(Clone, Debug)]
pub(super) struct Pattern {
    alternatives: Vec<Alternative>,
    cost: usize,
}

#[derive(Clone, Debug)]
struct Alternative {
    basename: bool,
    segments: Vec<Segment>,
}

#[derive(Clone, Debug)]
enum Segment {
    Recursive,
    Tokens(Vec<Token>),
}

#[derive(Clone, Debug)]
enum Token {
    Literal(char),
    One,
    Many,
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
}

impl Pattern {
    pub(super) fn new(pattern: &str) -> Option<Self> {
        let normalized = normalize(pattern);
        let chars = normalized.chars().collect::<Vec<_>>();
        if chars.is_empty() || chars.len() > MAX_PATTERN_CHARS {
            return None;
        }
        let mut expanded = Vec::new();
        expand(&chars, 0, &mut expanded)?;
        let cost = expanded.iter().map(Vec::len).sum::<usize>();
        if cost > 32 * 1024 {
            return None;
        }
        let alternatives = expanded
            .into_iter()
            .map(|chars| {
                let text = chars.into_iter().collect::<String>();
                let basename = !text.contains('/');
                let segments = text
                    .split('/')
                    .map(compile_segment)
                    .collect::<Option<Vec<_>>>()?;
                Some(Alternative { basename, segments })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self { alternatives, cost })
    }

    pub(super) fn cost(&self) -> usize {
        self.cost
    }

    /// Scan roots beneath a generated directory named literally in the glob.
    /// Descend through following literal directories without walking their
    /// siblings; a wildcard begins the scanned subtree.
    pub(super) fn explicit_scan_suffixes(&self, path: &str) -> Vec<PathBuf> {
        let normalized = normalize(path);
        let parts = normalized.split('/').collect::<Vec<_>>();
        let mut roots = Vec::new();
        for alternative in &self.alternatives {
            let mut reachable = vec![false; parts.len() + 1];
            reachable[0] = true;
            for (index, segment) in alternative.segments.iter().enumerate() {
                match segment {
                    Segment::Recursive => {
                        for index in 1..reachable.len() {
                            reachable[index] |= reachable[index - 1];
                        }
                    }
                    Segment::Tokens(tokens) => {
                        for index in (1..reachable.len()).rev() {
                            reachable[index] =
                                reachable[index - 1] && match_tokens(tokens, parts[index - 1]);
                        }
                        reachable[0] = false;
                        if index + 1 < alternative.segments.len()
                            && tokens
                                .iter()
                                .all(|token| matches!(token, Token::Literal(_)))
                            && reachable[parts.len()]
                        {
                            let remaining = &alternative.segments[index + 1..];
                            let literal = remaining
                                .iter()
                                .take_while(|segment| {
                                    matches!(segment, Segment::Tokens(tokens) if tokens.iter().all(|token| matches!(token, Token::Literal(_))))
                                })
                                .count();
                            let directories = if literal == remaining.len() {
                                literal.saturating_sub(1)
                            } else {
                                literal
                            };
                            let mut suffix = PathBuf::new();
                            for segment in &remaining[..directories] {
                                let Segment::Tokens(tokens) = segment else {
                                    unreachable!()
                                };
                                suffix.push(
                                    tokens
                                        .iter()
                                        .filter_map(|token| match token {
                                            Token::Literal(ch) => Some(*ch),
                                            _ => None,
                                        })
                                        .collect::<String>(),
                                );
                            }
                            roots.push(suffix);
                        }
                    }
                }
            }
        }
        roots
    }

    pub(super) fn matches(&self, path: &str) -> bool {
        if path.len() > 128 * 1024 {
            return false;
        }
        let normalized = normalize(path);
        let parts = normalized.split('/').collect::<Vec<_>>();
        self.alternatives.iter().any(|alternative| {
            if alternative.basename {
                let Segment::Tokens(tokens) = &alternative.segments[0] else {
                    return true;
                };
                return match_tokens(tokens, parts.last().copied().unwrap_or(""));
            }
            let mut reachable = vec![false; parts.len() + 1];
            reachable[0] = true;
            for segment in &alternative.segments {
                match segment {
                    Segment::Recursive => {
                        for index in 1..reachable.len() {
                            reachable[index] |= reachable[index - 1];
                        }
                    }
                    Segment::Tokens(tokens) => {
                        for index in (1..reachable.len()).rev() {
                            reachable[index] =
                                reachable[index - 1] && match_tokens(tokens, parts[index - 1]);
                        }
                        reachable[0] = false;
                    }
                }
            }
            reachable[parts.len()]
        })
    }
}

fn normalize(path: &str) -> String {
    let path = path.replace('\\', "/");
    if path
        .get(..8)
        .is_some_and(|head| head.eq_ignore_ascii_case("//?/UNC/"))
    {
        format!("//{}", &path[8..])
    } else if let Some(rest) = path.strip_prefix("//?/") {
        rest.to_owned()
    } else {
        path
    }
}

fn expand(input: &[char], groups: usize, output: &mut Vec<Vec<char>>) -> Option<()> {
    if groups > MAX_BRACE_GROUPS || input.len() > MAX_PATTERN_CHARS {
        return None;
    }
    let Some((open, close)) = first_brace(input)? else {
        if output.len() == MAX_ALTERNATIVES {
            return None;
        }
        output.push(input.to_vec());
        return Some(());
    };
    for choice in choices(&input[open + 1..close])? {
        let mut next = Vec::with_capacity(open + choice.len() + input.len() - close - 1);
        next.extend_from_slice(&input[..open]);
        next.extend_from_slice(choice);
        next.extend_from_slice(&input[close + 1..]);
        expand(&next, groups + 1, output)?;
    }
    Some(())
}

fn first_brace(input: &[char]) -> Option<Option<(usize, usize)>> {
    let mut open = None;
    let mut depth = 0;
    let mut class = false;
    for (index, ch) in input.iter().enumerate() {
        match ch {
            '[' if !class => class = true,
            ']' if class => class = false,
            _ if class => {}
            '{' => {
                open.get_or_insert(index);
                depth += 1;
                if depth > MAX_BRACE_GROUPS {
                    return None;
                }
            }
            '}' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                if depth == 0 {
                    return Some(Some((open?, index)));
                }
            }
            _ => {}
        }
    }
    if depth != 0 || class {
        None
    } else {
        Some(None)
    }
}

fn choices(input: &[char]) -> Option<Vec<&[char]>> {
    let mut choices = Vec::new();
    let mut depth = 0;
    let mut class = false;
    let mut start = 0;
    for (index, ch) in input.iter().enumerate() {
        match ch {
            '[' if !class => class = true,
            ']' if class => class = false,
            _ if class => {}
            '{' => depth += 1,
            '}' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
            }
            ',' if depth == 0 => {
                choices.push(&input[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if depth != 0 || class {
        return None;
    }
    choices.push(&input[start..]);
    Some(choices)
}

fn compile_segment(text: &str) -> Option<Segment> {
    if text == "**" {
        return Some(Segment::Recursive);
    }
    let chars = text.chars().collect::<Vec<_>>();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '*' => {
                if !matches!(tokens.last(), Some(Token::Many)) {
                    tokens.push(Token::Many);
                }
                index += 1;
            }
            '?' => {
                tokens.push(Token::One);
                index += 1;
            }
            '[' => tokens.push(parse_class(&chars, &mut index)?),
            ch => {
                tokens.push(Token::Literal(ch));
                index += 1;
            }
        }
    }
    Some(Segment::Tokens(tokens))
}

fn parse_class(chars: &[char], index: &mut usize) -> Option<Token> {
    let mut cursor = *index + 1;
    let negated = chars.get(cursor) == Some(&'!');
    if negated {
        cursor += 1;
    }
    let mut ranges = Vec::new();
    while let Some(&first) = chars.get(cursor) {
        if first == ']' {
            break;
        }
        if chars.get(cursor + 1) == Some(&'-') && chars.get(cursor + 2).is_some_and(|ch| *ch != ']')
        {
            let last = chars[cursor + 2];
            if first > last {
                return None;
            }
            ranges.push((first, last));
            cursor += 3;
        } else {
            ranges.push((first, first));
            cursor += 1;
        }
    }
    if ranges.is_empty() || chars.get(cursor) != Some(&']') {
        return None;
    }
    *index = cursor + 1;
    Some(Token::Class { negated, ranges })
}

fn match_tokens(tokens: &[Token], text: &str) -> bool {
    let chars = text.chars().collect::<Vec<_>>();
    let mut reachable = vec![false; chars.len() + 1];
    reachable[0] = true;
    for token in tokens {
        if matches!(token, Token::Many) {
            for index in 1..reachable.len() {
                reachable[index] |= reachable[index - 1];
            }
        } else {
            for index in (1..reachable.len()).rev() {
                reachable[index] = reachable[index - 1] && token_matches(token, chars[index - 1]);
            }
            reachable[0] = false;
        }
    }
    reachable[chars.len()]
}

fn token_matches(token: &Token, ch: char) -> bool {
    match token {
        Token::One => true,
        Token::Many => unreachable!(),
        Token::Literal(expected) => same(*expected, ch),
        Token::Class { negated, ranges } => {
            let included = ranges.iter().any(|&(first, last)| {
                (first <= ch && ch <= last)
                    || (first == last && same(first, ch))
                    || ascii_folded_range(first, last, ch)
            });
            included != *negated
        }
    }
}

fn same(first: char, second: char) -> bool {
    #[cfg(windows)]
    {
        first == second || first.to_lowercase().eq(second.to_lowercase())
    }
    #[cfg(not(windows))]
    {
        first == second
    }
}

fn ascii_folded_range(first: char, last: char, ch: char) -> bool {
    #[cfg(windows)]
    {
        first.is_ascii()
            && last.is_ascii()
            && ch.is_ascii()
            && first.to_ascii_lowercase() <= ch.to_ascii_lowercase()
            && ch.to_ascii_lowercase() <= last.to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        let _ = (first, last, ch);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::Pattern;

    fn matches(pattern: &str, path: &str) -> bool {
        Pattern::new(pattern).unwrap().matches(path)
    }

    #[test]
    fn segments_and_unicode_characters() {
        assert!(matches("src/**/*.rs", "src/lib.rs"));
        assert!(matches("src/**/*.rs", "src/a/b/lib.rs"));
        assert!(!matches("src/*.rs", "src/a/lib.rs"));
        assert!(matches("?.rs", "src/🦀.rs"));
        assert!(!matches("?.rs", "src/🦀🦀.rs"));
        assert!(matches("!*.rs", "src/!main.rs"));
        assert!(!matches("!*.rs", "src/main.rs"));
    }

    #[test]
    fn nested_braces_and_character_classes() {
        assert!(matches("src/{lib,{main,test}}.[rR][sS]", "src/test.rs"));
        assert!(matches("src/{lib,{main,test}}.[rR][sS]", "src/main.RS"));
        assert!(!matches("src/{lib,{main,test}}.[rR][sS]", "src/other.rs"));
        assert!(matches("src/[a-z]oo.rs", "src/boo.rs"));
        assert!(matches("src/[!a-z]oo.rs", "src/7oo.rs"));
        assert!(!matches("src/[!a-z]oo.rs", "src/boo.rs"));
    }

    #[test]
    fn absolute_paths_and_windows_verbatim_prefixes() {
        assert!(matches("/home/user/**/*.rs", "/home/user/lib.rs"));
        assert!(matches(r"C:\work\**\*.rs", r"\\?\C:\work\src\lib.rs"));
        assert!(matches(
            r"\\server\share\**\*.rs",
            r"\\?\UNC\server\share\lib.rs"
        ));
        assert!(!matches("src/**/*.rs", "/home/user/src/lib.rs"));
        #[cfg(windows)]
        assert!(matches(r"c:\WORK\*.RS", r"C:\work\lib.rs"));
    }

    #[test]
    fn invalid_and_excessive_patterns_are_rejected() {
        for invalid in [
            "",
            "src/{lib,main.rs",
            "src/lib}.rs",
            "src/[a-z.rs",
            "src/[z-a].rs",
            "src/[]",
        ] {
            assert!(Pattern::new(invalid).is_none(), "{invalid}");
        }
        assert!(Pattern::new(&"a".repeat(4097)).is_none());
        assert!(Pattern::new(&format!("{}x", "{a,b}".repeat(8))).is_none());
        assert!(Pattern::new(&format!("{}{}", "x".repeat(3900), "{a,b}".repeat(5))).is_none());
        assert!(Pattern::new(&format!("{}x", "{a}".repeat(33))).is_none());
    }
}
