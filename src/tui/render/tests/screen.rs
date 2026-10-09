use crate::tui::text;

struct Saved {
    rows: Vec<Vec<char>>,
    history: Vec<String>,
    x: usize,
    y: usize,
}

// An owned terminal model: alternate buffers, cursor movement, erasure and
// delayed wrapping and line feeds. It has no knowledge of Jecode's layout or cache.
pub(super) struct Screen {
    pub rows: Vec<Vec<char>>,
    pub history: Vec<String>,
    pub x: usize,
    pub y: usize,
    main: Option<Saved>,
}

impl Screen {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            rows: vec![vec![' '; width]; height],
            history: vec![],
            x: 0,
            y: 0,
            main: None,
        }
    }

    pub fn enter(&mut self) {
        if self.main.is_none() {
            self.feed("\x1b[?1049h");
        }
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        self.rows.resize(height, vec![' '; width]);
        for row in &mut self.rows {
            row.resize(width, ' ');
        }
        self.x = self.x.min(width - 1);
        self.y = self.y.min(height - 1);
    }

    pub fn feed(&mut self, value: &str) {
        let mut chars = value.chars();
        while let Some(character) = chars.next() {
            match character {
                '\x1b' => {
                    assert_eq!(chars.next(), Some('['));
                    let mut code = String::new();
                    let end = loop {
                        let next = chars.next().unwrap();
                        if next.is_ascii_alphabetic() {
                            break next;
                        }
                        code.push(next);
                    };
                    match end {
                        'h' if code == "?1049" && self.main.is_none() => {
                            let width = self.rows[0].len();
                            let height = self.rows.len();
                            self.main = Some(Saved {
                                rows: std::mem::replace(
                                    &mut self.rows,
                                    vec![vec![' '; width]; height],
                                ),
                                history: std::mem::take(&mut self.history),
                                x: self.x,
                                y: self.y,
                            });
                            self.x = 0;
                            self.y = 0;
                        }
                        'l' if code == "?1049" => {
                            if let Some(main) = self.main.take() {
                                self.rows = main.rows;
                                self.history = main.history;
                                self.x = main.x;
                                self.y = main.y;
                            }
                        }
                        'H' => {
                            let parts: Vec<_> = code
                                .split(';')
                                .map(|part| part.parse::<usize>().unwrap_or(1))
                                .collect();
                            self.y = parts[0].saturating_sub(1).min(self.rows.len() - 1);
                            self.x = parts
                                .get(1)
                                .copied()
                                .unwrap_or(1)
                                .saturating_sub(1)
                                .min(self.rows[0].len() - 1);
                        }
                        'K' => self.rows[self.y].fill(' '),
                        'J' if code == "3" => self.history.clear(),
                        'J' if code == "2" => {
                            for row in &mut self.rows {
                                row.fill(' ');
                            }
                        }
                        'J' => {
                            self.rows[self.y][self.x..].fill(' ');
                            for row in &mut self.rows[self.y + 1..] {
                                row.fill(' ');
                            }
                        }
                        'm' | 'h' | 'l' => {}
                        _ => panic!("unsupported sequence {code}{end}"),
                    }
                }
                '\r' => self.x = 0,
                '\n' => {
                    self.x = self.x.min(self.rows[0].len() - 1);
                    self.line_feed();
                }
                character => {
                    let size = text::width(character);
                    if size > 0 && self.x + size > self.rows[0].len() {
                        self.x = 0;
                        self.line_feed();
                    }
                    if size > 0 && self.x < self.rows[0].len() {
                        self.rows[self.y][self.x] = character;
                    }
                    self.x += size;
                }
            }
        }
    }

    fn line_feed(&mut self) {
        if self.y + 1 == self.rows.len() {
            let removed = self.rows.remove(0).into_iter().collect();
            if self.main.is_none() {
                self.history.push(removed);
            }
            self.rows.push(vec![' '; self.rows[0].len()]);
        } else {
            self.y += 1;
        }
    }

    pub fn visible(&self) -> String {
        self.rows
            .iter()
            .map(|row| row.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }
}
