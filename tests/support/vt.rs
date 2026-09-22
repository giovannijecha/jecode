//! Owned display oracle for the VT subset emitted by the Windows console host.
#[derive(Clone, Default)]
struct Line {
    text: Vec<char>,
    wrapped: bool,
}
pub struct Screen {
    width: usize,
    height: usize,
    rows: Vec<Line>,
    history: Vec<Line>,
    row: usize,
    column: usize,
    pending: String,
}
impl Screen {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            rows: vec![Line::default(); height],
            history: Vec::new(),
            row: 0,
            column: 0,
            pending: String::new(),
        }
    }
    pub fn text(&self) -> String {
        self.history
            .iter()
            .chain(&self.rows)
            .map(|line| line.text.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub fn resize(&mut self, width: usize, height: usize) {
        let cursor_row = self.history.len() + self.row;
        let mut all = std::mem::take(&mut self.history);
        all.extend(std::mem::take(&mut self.rows));
        let last = all
            .iter()
            .rposition(|line| line.text.iter().any(|ch| *ch != ' '))
            .unwrap_or(0)
            .max(cursor_row);
        all.truncate(last + 1);
        let mut result = Vec::new();
        let mut logical = Vec::new();
        let mut cursor = None;
        let mut mapped = (0, 0);
        for (index, line) in all.into_iter().enumerate() {
            if index == cursor_row {
                cursor = Some(logical.len() + self.column);
            }
            logical.extend(line.text);
            if line.wrapped {
                continue;
            }
            let base = result.len();
            if logical.is_empty() {
                result.push(Line::default());
            } else {
                let count = logical.len().div_ceil(width);
                for (index, chunk) in logical.chunks(width).enumerate() {
                    result.push(Line {
                        text: chunk.to_vec(),
                        wrapped: index + 1 < count,
                    });
                }
            }
            if let Some(offset) = cursor.take() {
                mapped = (
                    base + (offset / width).min(result.len() - base - 1),
                    offset % width,
                );
            }
            logical.clear();
        }
        let start = result.len().saturating_sub(height);
        self.rows = result.split_off(start);
        self.history = result;
        self.rows.resize(height, Line::default());
        self.row = mapped.0.saturating_sub(start).min(height - 1);
        self.column = mapped.1;
        self.width = width;
        self.height = height;
    }
    fn down(&mut self) {
        self.row += 1;
        if self.row >= self.height {
            self.history.push(self.rows.remove(0));
            self.rows.push(Line::default());
            self.row = self.height - 1;
        }
    }
    pub fn feed(&mut self, input: &str) {
        self.pending.push_str(input);
        let text = std::mem::take(&mut self.pending);
        let mut rest = text.as_str();
        while !rest.is_empty() {
            if let Some(sequence) = rest.strip_prefix("\x1b[") {
                let Some(end) = sequence.find(|c: char| ('@'..='~').contains(&c)) else {
                    break;
                };
                let args = &sequence[..end];
                let p: Vec<usize> = args.split(';').map(|s| s.parse().unwrap_or(0)).collect();
                let n = p[0];
                match sequence.as_bytes()[end] {
                    b'H' | b'f' => {
                        self.row = n.max(1).saturating_sub(1).min(self.height - 1);
                        self.column = p
                            .get(1)
                            .copied()
                            .unwrap_or(1)
                            .max(1)
                            .saturating_sub(1)
                            .min(self.width - 1);
                    }
                    b'A' => self.row = self.row.saturating_sub(n.max(1)),
                    b'B' => self.row = (self.row + n.max(1)).min(self.height - 1),
                    b'C' => self.column = (self.column + n.max(1)).min(self.width - 1),
                    b'D' => self.column = self.column.saturating_sub(n.max(1)),
                    b'G' => self.column = n.max(1).saturating_sub(1).min(self.width - 1),
                    b'J' if n == 0 => {
                        self.rows[self.row].text.truncate(self.column);
                        self.rows[self.row + 1..].fill(Line::default());
                    }
                    b'J' if n == 2 => {
                        self.history
                            .extend(self.rows.iter().filter(|r| !r.text.is_empty()).cloned());
                        self.rows.fill(Line::default());
                    }
                    b'K' if n == 0 => self.rows[self.row].text.truncate(self.column),
                    b'K' if n == 2 => self.rows[self.row] = Line::default(),
                    b'X' => {
                        let row = &mut self.rows[self.row].text;
                        row.resize(row.len().max((self.column + n.max(1)).min(self.width)), ' ');
                        let end = (self.column + n.max(1)).min(row.len());
                        row[self.column..end].fill(' ');
                    }
                    b't' if n == 8 => {
                        let (height, width) = (p[1], p[2]);
                        if (width, height) != (self.width, self.height) {
                            self.resize(width, height);
                        }
                    }
                    b'm' | b'h' | b'l' => {}
                    other => panic!("unhandled VT: {args}{}", other as char),
                }
                rest = &sequence[end + 1..];
                continue;
            }
            if let Some(sequence) = rest.strip_prefix("\x1b]") {
                let Some(end) = sequence.find('\x07') else {
                    break;
                };
                rest = &sequence[end + 1..];
                continue;
            }
            let ch = rest.chars().next().unwrap();
            if ch == '\x1b' && rest.len() == 1 {
                break;
            }
            rest = &rest[ch.len_utf8()..];
            match ch {
                '\r' => self.column = 0,
                '\n' => {
                    self.rows[self.row].wrapped = false;
                    self.down();
                }
                '\x08' => self.column = self.column.saturating_sub(1),
                '\x07' => {}
                ch => {
                    assert!(!ch.is_control(), "unknown control {ch:?}");
                    if self.column == self.width {
                        self.rows[self.row].wrapped = true;
                        self.down();
                        self.column = 0;
                    }
                    let row = &mut self.rows[self.row].text;
                    row.resize(row.len().max(self.column + 1), ' ');
                    row[self.column] = ch;
                    self.column += 1;
                }
            }
        }
        self.pending.push_str(rest);
    }
}
