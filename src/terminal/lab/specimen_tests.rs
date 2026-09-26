//! Frozen style specimen: every tone, glyph set, tree and clipping case.
//!
//! Line format: `tone: text` · `@caps` · `@glyphs` · `@clip text` ·
//! `@wrap text` · blank line · `# comment`. `\u{...}` escapes are expanded.
use super::caps::Caps;
use super::glyph::{Glyphs, glyphs};
use super::style::{self, Row, Tone};
use super::text;

/// Inner width of the demonstration boxes used by `@clip` and `@wrap`.
const BOX: usize = 40;

pub fn rows(source: &str, caps: &Caps, width: usize) -> Result<Vec<Row>, String> {
    let glyph = glyphs(caps.ascii);
    let mut out = Vec::new();
    for (index, raw) in source.lines().enumerate() {
        let line =
            unescape(raw.trim_end()).map_err(|error| format!("line {}: {error}", index + 1))?;
        if line.starts_with('#') {
            continue;
        }
        if line.is_empty() {
            out.push(Row::blank());
        } else if line == "@caps" {
            out.push(caps_row(caps, glyph));
        } else if line == "@glyphs" {
            out.extend(glyph_rows(glyph, width));
        } else if let Some(sample) = line.strip_prefix("@clip ") {
            out.push(boxed(&text::clip(sample, BOX, glyph.ellipsis), glyph));
        } else if let Some(sample) = line.strip_prefix("@wrap ") {
            out.extend(text::wrap(sample, BOX).iter().map(|row| boxed(row, glyph)));
        } else {
            let (name, body) = line
                .split_once(": ")
                .ok_or_else(|| format!("line {}: expected `tone: text`", index + 1))?;
            let tone =
                tone(name).ok_or_else(|| format!("line {}: unknown tone `{name}`", index + 1))?;
            for row in lines(body, width, tone) {
                out.push(
                    if matches!(
                        tone,
                        Tone::User | Tone::Code | Tone::Added | Tone::Removed | Tone::Cursor
                    ) {
                        style::pad(row, width)
                    } else {
                        row
                    },
                );
            }
        }
    }
    Ok(out)
}

fn lines(value: &str, width: usize, tone: Tone) -> Vec<Row> {
    text::wrap(value, width)
        .into_iter()
        .map(|line| Row::new(line, tone))
        .collect()
}

fn caps_row(caps: &Caps, glyph: &Glyphs) -> Row {
    let dot = format!(" {} ", glyph.dot);
    let set = if caps.ascii { "ascii" } else { "unicode" };
    let motion = if caps.reduced_motion {
        "reduced motion"
    } else {
        "motion"
    };
    let color = match caps.color {
        super::caps::ColorDepth::None => "no color",
        super::caps::ColorDepth::Ansi16 => "16 colors",
        super::caps::ColorDepth::Ansi256 => "256 colors",
        super::caps::ColorDepth::TrueColor => "truecolor",
    };
    Row::new([color, set, motion].join(&dot), Tone::Muted)
}

fn glyph_rows(glyph: &Glyphs, width: usize) -> Vec<Row> {
    let dot = format!(" {} ", glyph.dot);
    let mut set = Row::new("", Tone::Text);
    for (label, symbol, tone) in [
        ("ok", glyph.ok, Tone::Success),
        ("fail", glyph.fail, Tone::Error),
        ("warn", glyph.warn, Tone::Warning),
        ("prompt", glyph.prompt, Tone::Accent),
        ("more", glyph.ellipsis, Tone::Text),
    ] {
        set.push(label, Tone::Muted)
            .push(" ", Tone::Text)
            .push(symbol, tone)
            .push("   ", Tone::Text);
    }
    let mut read = Row::new("", Tone::Text);
    read.push(glyph.branch, Tone::Muted)
        .push(" ", Tone::Text)
        .push(glyph.ok, Tone::Success)
        .push(" Read 6 files", Tone::Text)
        .push(&format!("{dot}src/session/{dot}0.4s"), Tone::Muted);
    let mut detail = Row::new("", Tone::Text);
    detail
        .push(glyph.stem, Tone::Muted)
        .push("    ", Tone::Text)
        .push("41", Tone::Muted)
        .push(" ", Tone::Text)
        .push("- pub struct SessionStore {", Tone::Removed);
    let mut run = Row::new("", Tone::Text);
    run.push(glyph.last, Tone::Muted)
        .push(" ", Tone::Text)
        .push(glyph.fail, Tone::Error)
        .push(" Run cargo clippy", Tone::Text)
        .push(&format!("{dot}exit 101{dot}8.1s"), Tone::Muted);
    vec![
        set,
        Row::new(glyph.rule.repeat(width), Tone::Accent),
        read,
        detail,
        run,
    ]
}

fn boxed(content: &str, glyph: &Glyphs) -> Row {
    let mut row = Row::new("", Tone::Text);
    row.push(glyph.stem, Tone::Muted)
        .push(&text::pad(content, BOX), Tone::Text)
        .push(glyph.stem, Tone::Muted);
    row
}

fn tone(name: &str) -> Option<Tone> {
    Some(match name {
        "text" => Tone::Text,
        "muted" => Tone::Muted,
        "accent" => Tone::Accent,
        "user" => Tone::User,
        "code" => Tone::Code,
        "added" => Tone::Added,
        "removed" => Tone::Removed,
        "heading" => Tone::Heading,
        "error" => Tone::Error,
        "warning" => Tone::Warning,
        "success" => Tone::Success,
        "keyword" => Tone::Keyword,
        "string" => Tone::String,
        "number" => Tone::Number,
        "cursor" => Tone::Cursor,
        _ => return None,
    })
}

fn unescape(line: &str) -> Result<String, String> {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find("\\u{") {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 3..];
        let close = tail.find('}').ok_or("unclosed \\u{")?;
        let ch = u32::from_str_radix(&tail[..close], 16)
            .ok()
            .and_then(char::from_u32)
            .ok_or_else(|| format!("bad escape \\u{{{}}}", &tail[..close]))?;
        out.push(ch);
        rest = &tail[close + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::caps::ColorDepth;
    use super::super::render::Renderer;
    use super::*;

    const CAPS: Caps = Caps {
        color: ColorDepth::None,
        ascii: false,
        reduced_motion: false,
    };

    #[test]
    fn every_clip_box_has_the_same_width() {
        let source = "@clip plain ascii that is long enough to overflow the box\n\
                      @clip e\\u{301} composed · 👨\\u{200d}👩\\u{200d}👧 · 日本語のテキストとか 🇮🇹 end\n\
                      @clip short";
        let rows = rows(source, &CAPS, 80).unwrap();
        for row in rows {
            assert_eq!(text::width(&row.text), BOX + 2, "{:?}", row.text);
        }
    }

    #[test]
    fn errors_name_the_line() {
        assert_eq!(
            rows("\nnope: x", &CAPS, 80).unwrap_err(),
            "line 2: unknown tone `nope`"
        );
        assert_eq!(
            rows("bare", &CAPS, 80).unwrap_err(),
            "line 1: expected `tone: text`"
        );
    }

    #[test]
    fn ascii_set_leaves_no_box_drawing() {
        let caps = Caps {
            ascii: true,
            ..CAPS
        };
        let rows = rows("@glyphs", &caps, 20).unwrap();
        assert!(rows.iter().all(|row| row.text.is_ascii()), "{rows:?}");
    }

    #[test]
    fn frozen_lab_style_specimen_matches_every_color_fallback() {
        const SOURCE: &str = include_str!("../../../tests/fixtures/tui/specimen.txt");
        let cases: &[(usize, ColorDepth, bool, bool, &[u8])] = &[
            (
                80,
                ColorDepth::TrueColor,
                false,
                false,
                include_bytes!(
                    "../../../tests/fixtures/tui/specimen-80-truecolor-unicode-normal.ansi"
                ),
            ),
            (
                80,
                ColorDepth::Ansi256,
                false,
                false,
                include_bytes!("../../../tests/fixtures/tui/specimen-80-256-unicode-normal.ansi"),
            ),
            (
                80,
                ColorDepth::Ansi16,
                false,
                false,
                include_bytes!("../../../tests/fixtures/tui/specimen-80-16-unicode-normal.ansi"),
            ),
            (
                120,
                ColorDepth::None,
                true,
                true,
                include_bytes!("../../../tests/fixtures/tui/specimen-120-none-ascii-reduced.ansi"),
            ),
        ];
        for &(width, color, ascii, reduced_motion, expected) in cases {
            let caps = Caps {
                color,
                ascii,
                reduced_motion,
            };
            let frame = rows(SOURCE, &caps, width).unwrap();
            let height = frame.len() + 1;
            let output = Renderer::default().draw(frame, (width, height), color);
            let output = output
                .replace("\x1b[?2026h\x1b[?7l", "\x1b[?2026h")
                .replace("\x1b[?7h\x1b[?2026l", "\x1b[?2026l");
            assert_eq!(output.as_bytes(), expected, "{width} columns, {caps:?}");
        }
    }
}
