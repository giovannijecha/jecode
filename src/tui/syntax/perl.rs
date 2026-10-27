// Perl's patterns and quote-like operators read as one string, so a stray
// apostrophe inside `s/harness's/x/` never opens a quote.

/// The quote-like operators and how many delimited parts each reads.
const OPERATORS: &[(&str, usize)] = &[
    ("qr", 1),
    ("qq", 1),
    ("qw", 1),
    ("qx", 1),
    ("tr", 2),
    ("m", 1),
    ("q", 1),
    ("s", 2),
    ("y", 2),
];
/// Words after which a bare `/` opens a pattern instead of dividing.
const PATTERN_WORDS: &[&str] = &[
    "split", "grep", "map", "join", "return", "and", "or", "not", "if", "unless", "while", "until",
    "when",
];

/// An open pattern or quote-like body, carried across lines.
#[derive(Clone, Copy)]
pub(super) struct Delimited {
    open: char,
    close: char,
    depth: usize,
    // Parts still to read after this one, as the replacement of `s///`.
    parts: usize,
}

/// A pattern or quote-like literal at the start of `tail`: its length on this
/// line, and its state when it continues on the next one.
pub(super) fn literal(tail: &str, before: &str) -> Option<(usize, Option<Delimited>)> {
    let (operator, parts) = if tail.starts_with('/') {
        pattern_position(before).then_some((0, 1))?
    } else {
        // A sigil, method arrow or package separator makes it a name.
        if before
            .chars()
            .next_back()
            .is_some_and(|character| character.is_alphanumeric() || "_$@%&*>:".contains(character))
        {
            return None;
        }
        OPERATORS
            .iter()
            .find(|(name, _)| {
                tail.starts_with(name) && tail[name.len()..].chars().next().is_some_and(delimiter)
            })
            .map(|&(name, parts)| (name.len(), parts))?
    };
    let open = tail[operator..].chars().next()?;
    let start = operator + open.len_utf8();
    let (length, state) = body(
        &tail[start..],
        Delimited {
            open,
            close: partner(open),
            depth: 1,
            parts: parts - 1,
        },
    );
    Some((start + length, state))
}

/// The rest of a delimited body from inside it, then any trailing flags.
pub(super) fn body(value: &str, mut state: Delimited) -> (usize, Option<Delimited>) {
    let mut position = 0;
    loop {
        let mut escaped = false;
        let mut closed = false;
        for (index, character) in value[position..].char_indices() {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == state.close {
                state.depth -= 1;
                if state.depth == 0 {
                    position += index + character.len_utf8();
                    closed = true;
                    break;
                }
            } else if character == state.open {
                state.depth += 1;
            }
        }
        if !closed {
            return (value.len(), Some(state));
        }
        if state.parts == 0 {
            break;
        }
        state.parts -= 1;
        state.depth = 1;
        // `s{a}{b}` opens its second part with its own delimiters.
        if state.open != state.close {
            let rest = value[position..].trim_start();
            let Some(open) = rest.chars().next().filter(|&open| delimiter(open)) else {
                break;
            };
            position = value.len() - rest.len() + open.len_utf8();
            state.open = open;
            state.close = partner(open);
        }
    }
    position += value[position..]
        .bytes()
        .take_while(u8::is_ascii_alphabetic)
        .count();
    (position, None)
}

fn delimiter(character: char) -> bool {
    !character.is_alphanumeric()
        && !character.is_whitespace()
        && !matches!(character, '_' | ',' | ';' | ')' | '=' | '>')
}

fn partner(open: char) -> char {
    match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '<' => '>',
        other => other,
    }
}

/// A `/` opens a pattern at the start of a statement or argument, after a
/// binding `=~` or `!~`, or after a word such as `split`.
fn pattern_position(before: &str) -> bool {
    let trimmed = before.trim_end();
    let word = &trimmed[trimmed
        .trim_end_matches(|character: char| character.is_alphanumeric() || character == '_')
        .len()..];
    match trimmed.chars().next_back() {
        None => true,
        Some(character) if "(,;{=~!&|?:".contains(character) => true,
        Some(_) => PATTERN_WORDS.contains(&word),
    }
}
