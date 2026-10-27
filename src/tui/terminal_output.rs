//! Converts PTY control output to a readable, safe TUI transcript. Saved
//! bytes remain raw. Cursor-addressed full-screen programs remain logs.

pub(super) fn transcript(raw: &str) -> String {
    const GROUND: u8 = 0;
    const ESCAPE: u8 = 1;
    const CSI: u8 = 2;
    const OSC: u8 = 3;
    const OSC_ESCAPE: u8 = 4;
    const ESCAPE_INTERMEDIATE: u8 = 5;
    const CSI_INTERMEDIATE: u8 = 6;

    let mut output = String::with_capacity(raw.len());
    let mut line = Vec::<char>::new();
    let mut cursor: usize = 0;
    let mut state = GROUND;
    let mut csi = String::new();
    for character in raw.chars() {
        match state {
            GROUND => match character {
                '\x1b' => state = ESCAPE,
                '\n' => {
                    output.extend(line.drain(..));
                    output.push('\n');
                    cursor = 0;
                }
                '\r' => cursor = 0,
                '\x08' => cursor = cursor.saturating_sub(1),
                '\t' => {
                    let prefix: String = line.iter().take(cursor).collect();
                    let spaces = 8 - super::text::cells(&prefix) % 8;
                    for _ in 0..spaces {
                        put(&mut line, &mut cursor, ' ');
                    }
                }
                value if !value.is_control() && !('\u{7f}'..='\u{9f}').contains(&value) => {
                    put(&mut line, &mut cursor, value);
                }
                _ => {}
            },
            ESCAPE => {
                state = match character {
                    '[' => {
                        csi.clear();
                        CSI
                    }
                    ']' => OSC,
                    ' '..='/' => ESCAPE_INTERMEDIATE,
                    '\x1b' => ESCAPE,
                    _ => GROUND,
                };
            }
            ESCAPE_INTERMEDIATE => {
                state = match character {
                    ' '..='/' => ESCAPE_INTERMEDIATE,
                    '0'..='~' => GROUND,
                    '\x1b' => ESCAPE,
                    _ => GROUND,
                };
            }
            CSI => {
                if ('\u{40}'..='\u{7e}').contains(&character) {
                    if character == 'K' {
                        match csi.as_str() {
                            "" | "0" => line.truncate(cursor),
                            "2" => {
                                line.clear();
                                cursor = 0;
                            }
                            _ => {}
                        }
                    }
                    state = GROUND;
                } else if ('\u{30}'..='\u{3f}').contains(&character) && csi.len() < 128 {
                    csi.push(character);
                } else if ('\u{20}'..='\u{2f}').contains(&character) {
                    state = CSI_INTERMEDIATE;
                } else if character == '\x1b' {
                    state = ESCAPE;
                } else if !('\u{30}'..='\u{3f}').contains(&character) {
                    state = GROUND;
                }
            }
            CSI_INTERMEDIATE => {
                state = match character {
                    ' '..='/' => CSI_INTERMEDIATE,
                    '@'..='~' => GROUND,
                    '\x1b' => ESCAPE,
                    _ => GROUND,
                };
            }
            OSC => {
                if character == '\x07' {
                    state = GROUND;
                } else if character == '\x1b' {
                    state = OSC_ESCAPE;
                }
            }
            OSC_ESCAPE => {
                state = if character == '\\' { GROUND } else { OSC };
            }
            _ => unreachable!(),
        }
    }
    output.extend(line);
    output
}

fn put(line: &mut Vec<char>, cursor: &mut usize, character: char) {
    if *cursor < line.len() {
        line[*cursor] = character;
    } else {
        line.push(character);
    }
    *cursor += 1;
}

#[cfg(test)]
mod tests {
    use super::transcript;

    #[test]
    fn strips_vt_controls_and_applies_current_line_edits() {
        let raw = "\x1b[?25l\x1b]0;private title\x07Hello\rBye\x08?\x1b[31m\x1b[K\nnext\x1b[0m\n";
        assert_eq!(transcript(raw), "By?\nnext\n");
    }

    #[test]
    fn strips_sequences_split_across_saved_pages() {
        let pages = ["\x1b]0;ti", "tle\x1b\\ready\x1b[", "2J\rDONE\n"];
        assert_eq!(transcript(&pages.concat()), "DONEy\n");
    }

    #[test]
    fn long_osc_and_csi_remain_suppressed_until_their_terminators() {
        let raw = format!(
            "before\x1b]0;{}\x07middle\x1b[{}mend\n",
            "private".repeat(700),
            "1".repeat(300)
        );
        assert_eq!(transcript(&raw), "beforemiddleend\n");
    }

    #[test]
    fn escape_intermediates_consume_their_final_byte() {
        assert_eq!(transcript("\x1b(Bplain\x1b)0\x1b#8end\n"), "plainend\n");
        assert_eq!(transcript("\x1b(B\x1b[0m\x1b[?25l\x1b[1 qready"), "ready");
    }

    #[test]
    fn tabs_use_eight_cell_stops_after_wide_and_combining_glyphs() {
        assert_eq!(
            transcript("a\tX\n界\tY\ne\u{301}\tZ\n"),
            "a       X\n界      Y\ne\u{301}       Z\n"
        );
        assert_eq!(transcript("12345678\tX\n"), "12345678        X\n");
    }

    #[test]
    fn controls_preserve_overwrites_and_hide_osc_payloads() {
        let raw = "old\rnew\x1b[K\nAB\x08!\n\x1b]0;private\x1b\\visible\n";
        assert_eq!(transcript(raw), "new\nA!\nvisible\n");
    }
}
