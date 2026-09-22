//! Small VT test surface including Windows' preserve-on-ED2 scrollback behavior.
//! This is a regression oracle for emitted effects, not a general terminal emulator.
use super::{
    render::Renderer,
    style::{Row, Tone},
};

struct Surface {
    rows: Vec<String>,
    history: Vec<String>,
    cursor: (usize, usize),
}
impl Surface {
    fn new(height: usize) -> Self {
        Self {
            rows: vec![String::new(); height],
            history: Vec::new(),
            cursor: (0, 0),
        }
    }
    fn feed(&mut self, bytes: &str) {
        let mut rest = bytes;
        while !rest.is_empty() {
            if let Some(sequence) = rest.strip_prefix("\x1b[") {
                let end = sequence.find(|c: char| c.is_ascii_alphabetic()).unwrap();
                let args = &sequence[..end];
                let first = args
                    .split(';')
                    .next()
                    .unwrap()
                    .parse::<usize>()
                    .unwrap_or(0);
                match sequence.as_bytes()[end] {
                    b'H' => {
                        let column = args
                            .split(';')
                            .nth(1)
                            .unwrap_or("1")
                            .parse::<usize>()
                            .unwrap();
                        self.cursor = (first.max(1) - 1, column.max(1) - 1);
                    }
                    b'A' => self.cursor.0 = self.cursor.0.saturating_sub(first.max(1)),
                    b'J' if first == 2 => {
                        self.history
                            .extend(self.rows.iter().filter(|line| !line.is_empty()).cloned());
                        self.rows.fill(String::new());
                    }
                    b'J' if first == 0 => {
                        self.rows[self.cursor.0..].fill(String::new());
                    }
                    b'K' if first == 2 => self.rows[self.cursor.0].clear(),
                    _ => panic!(
                        "unsupported test escape: {args}{}",
                        sequence.as_bytes()[end] as char
                    ),
                }
                rest = &sequence[end + 1..];
                continue;
            }
            let ch = rest.chars().next().unwrap();
            rest = &rest[ch.len_utf8()..];
            match ch {
                '\r' => self.cursor.1 = 0,
                '\n' => {
                    self.cursor.0 += 1;
                    if self.cursor.0 == self.rows.len() {
                        self.history.push(self.rows.remove(0));
                        self.rows.push(String::new());
                        self.cursor.0 -= 1;
                    }
                }
                ch => {
                    assert!(ch.is_ascii(), "this fixture uses ASCII display cells");
                    let row = &mut self.rows[self.cursor.0];
                    if row.len() < self.cursor.1 {
                        row.push_str(&" ".repeat(self.cursor.1 - row.len()));
                    }
                    if row.len() > self.cursor.1 {
                        row.replace_range(self.cursor.1..self.cursor.1 + 1, &ch.to_string());
                    } else {
                        row.push(ch);
                    }
                    self.cursor.1 += 1;
                }
            }
        }
    }
}
fn frame(count: usize) -> Vec<Row> {
    (0..count)
        .map(|i| Row::new(format!("message-{i:03}"), Tone::Text))
        .chain(
            [
                Row::new("draft-kept", Tone::Text),
                Row::new("footer", Tone::Text),
            ]
            .map(|mut row| {
                row.transient = true;
                row
            }),
        )
        .collect()
}

#[test]
fn resize_does_not_archive_another_copy_of_the_visible_transcript() {
    for count in [5, 50] {
        let mut renderer = Renderer::default();
        let mut surface = Surface::new(24);
        surface.feed(&renderer.draw(frame(count), (80, 24), false));
        let history = surface.history.clone();
        for columns in [40, 120, 60, 80, 40, 120, 80] {
            surface.feed(&renderer.draw(frame(count), (columns, 24), false));
            assert_eq!(
                surface.history, history,
                "resize must not append existing rows to scrollback"
            );
            assert_eq!(
                surface
                    .rows
                    .iter()
                    .filter(|line| *line == "draft-kept")
                    .count(),
                1
            );
            assert_eq!(
                surface.rows.iter().filter(|line| *line == "footer").count(),
                1
            );
        }
        let next = frame(count + 1);
        surface.feed(&renderer.draw(next.clone(), (80, 24), false));
        assert!(
            surface
                .rows
                .iter()
                .any(|line| line == &format!("message-{count:03}"))
        );
        assert!(renderer.draw(next, (80, 24), false).is_empty());
    }
}
