use crate::tui::text;

struct Saved {
    rows: Vec<Vec<char>>,
    backs: Vec<Vec<Option<Rgb>>>,
    history: Vec<String>,
    x: usize,
    y: usize,
}

pub(super) type Rgb = (u8, u8, u8);

// An owned terminal model: alternate buffers, cursor movement, erasure,
// background colours and delayed wrapping and line feeds. Erasing fills with
// the default background, as on terminals without BCE. It has no knowledge
// of Jecode's layout or cache.
pub(super) struct Screen {
    pub rows: Vec<Vec<char>>,
    pub backs: Vec<Vec<Option<Rgb>>>,
    pub history: Vec<String>,
    pub x: usize,
    pub y: usize,
    background: Option<Rgb>,
    main: Option<Saved>,
}

impl Screen {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            rows: vec![vec![' '; width]; height],
            backs: vec![vec![None; width]; height],
            history: vec![],
            x: 0,
            y: 0,
            background: None,
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
        self.backs.resize(height, vec![None; width]);
        for row in &mut self.rows {
            row.resize(width, ' ');
        }
        for row in &mut self.backs {
            row.resize(width, None);
        }
        self.x = self.x.min(width - 1);
        self.y = self.y.min(height - 1);
    }

    fn graphics(&mut self, code: &str) {
        let parts: Vec<_> = code.split(';').collect();
        let mut index = 0;
        while index < parts.len() {
            match parts[index] {
                "" | "0" | "49" => self.background = None,
                "38" | "48" if parts.get(index + 1) == Some(&"2") => {
                    let channel = |offset: usize| parts[index + offset].parse::<u8>().unwrap();
                    if parts[index] == "48" {
                        self.background = Some((channel(2), channel(3), channel(4)));
                    }
                    index += 4;
                }
                _ => {}
            }
            index += 1;
        }
    }

    fn clear(&mut self, row: usize, from: usize) {
        self.rows[row][from..].fill(' ');
        self.backs[row][from..].fill(None);
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
                                backs: std::mem::replace(
                                    &mut self.backs,
                                    vec![vec![None; width]; height],
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
                                self.backs = main.backs;
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
                        'K' => self.clear(self.y, 0),
                        'J' if code == "3" => self.history.clear(),
                        'J' if code == "2" => {
                            for row in 0..self.rows.len() {
                                self.clear(row, 0);
                            }
                        }
                        'J' => {
                            self.clear(self.y, self.x);
                            for row in self.y + 1..self.rows.len() {
                                self.clear(row, 0);
                            }
                        }
                        'm' => self.graphics(&code),
                        'h' | 'l' => {}
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
                        let end = (self.x + size).min(self.rows[0].len());
                        self.backs[self.y][self.x..end].fill(self.background);
                    }
                    self.x += size;
                }
            }
        }
    }

    fn line_feed(&mut self) {
        if self.y + 1 == self.rows.len() {
            let removed = self.rows.remove(0).into_iter().collect();
            self.backs.remove(0);
            if self.main.is_none() {
                self.history.push(removed);
            }
            self.rows.push(vec![' '; self.rows[0].len()]);
            self.backs.push(vec![None; self.rows[0].len()]);
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

    /// Cells painted over the terminal's own background, as (row, column).
    pub fn painted(&self) -> Vec<(usize, usize)> {
        self.backs
            .iter()
            .enumerate()
            .flat_map(|(row, cells)| {
                cells
                    .iter()
                    .enumerate()
                    .filter(|(_, back)| back.is_some())
                    .map(move |(column, _)| (row, column))
            })
            .collect()
    }
}
