//! Converts PTY control output to a readable, safe log transcript. It does
//! not emulate a screen: cursor-addressed full-screen programs remain logs.

pub(super) fn transcript(raw: &str) -> String {
    const GROUND: u8 = 0;
    const ESCAPE: u8 = 1;
    const CSI: u8 = 2;
    const OSC: u8 = 3;
    const OSC_ESCAPE: u8 = 4;

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
                    for _ in 0..4 {
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
                } else if csi.len() < 128 {
                    csi.push(character);
                }
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
}
