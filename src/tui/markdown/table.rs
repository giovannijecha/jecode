use super::{horizontal_rule, inline, list_item, opening_fence};
use crate::tui::{
    line::{Line, Wrapped},
    text,
    theme::{BODY, EMPHASIS, MUTED},
};
use std::collections::VecDeque;

#[derive(Clone, Copy)]
enum Alignment {
    Left,
    Center,
    Right,
}

// Measure one source row per step, then lay out rows using the completed
// snapshot's widths. No whole-table buffer or body-wide scan inside one step.
pub(super) struct Table {
    headers: Vec<String>,
    alignments: Vec<Alignment>,
    widths: Vec<usize>,
    columns: usize,
    scan: usize,
    end: Option<usize>,
    emission: usize,
    stacked: bool,
    body_seen: bool,
    active: Option<Row>,
    rule: bool,
    origin: usize,
    character: usize,
}

enum Row {
    Columns(Vec<Wrapped>),
    Stacked {
        cells: VecDeque<Wrapped>,
        blank: bool,
    },
}

impl Table {
    pub fn new(
        header: &str,
        delimiter: &str,
        body: usize,
        columns: usize,
        origin: usize,
    ) -> Option<Self> {
        if block_start(header) {
            return None;
        }
        let (headers, pipes) = cells(header);
        let (delimiters, _) = cells(delimiter);
        if !pipes || headers.is_empty() || headers.len() != delimiters.len() {
            return None;
        }
        let alignments = delimiters
            .iter()
            .map(|cell| alignment(cell))
            .collect::<Option<Vec<_>>>()?;
        let widths = headers
            .iter()
            .map(|cell| visual_width(cell).max(2))
            .collect();
        Some(Self {
            headers,
            alignments,
            widths,
            columns,
            scan: body,
            end: None,
            emission: body,
            stacked: false,
            body_seen: false,
            active: None,
            rule: false,
            origin,
            character: 0,
        })
    }

    pub fn end(&self) -> usize {
        self.end.expect("table measurement completed")
    }

    pub fn step(&mut self, value: &str) -> Option<Vec<Line>> {
        if self.active.is_some() || self.rule {
            return Some(self.take_rows());
        }
        if self.end.is_none() {
            if let Some((source, next)) = source_line(value, self.scan)
                && !source.trim().is_empty()
                && !block_start(source)
            {
                let row = self.row(source);
                for (width, cell) in self.widths.iter_mut().zip(row) {
                    *width = (*width).max(visual_width(&cell));
                }
                self.scan = next;
                return Some(Vec::new());
            }
            self.end = Some(self.scan.min(value.len()));
            self.fit();
            if !self.stacked {
                self.active = Some(self.start_row(&self.headers, EMPHASIS, false));
                self.rule = true;
            } else if self.emission >= self.end() {
                self.active = Some(self.start_row(&self.headers, EMPHASIS, false));
            }
            return Some(self.take_rows());
        }
        if self.emission >= self.end() {
            return None;
        }
        let (source, next) = source_line(value, self.emission).expect("measured table row");
        self.origin = self.emission;
        self.character = 0;
        self.emission = next;
        let row = self.row(source);
        self.active = Some(self.start_row(&row, BODY, true));
        self.body_seen = true;
        Some(self.take_rows())
    }

    fn take_rows(&mut self) -> Vec<Line> {
        // Generate wrapped rows on demand; a tall cell never builds its full
        // visual output before yielding to the transcript's pass allowance.
        let mut output = Vec::new();
        while output.len() < 32 {
            if let Some(active) = &mut self.active {
                if let Some(line) = active.next(&self.widths, &self.alignments) {
                    let count = line
                        .plain()
                        .chars()
                        .filter(|ch| !ch.is_whitespace())
                        .count();
                    let mut line = line.at_source(self.origin);
                    line.origin.as_mut().unwrap().character = self.character;
                    self.character += count.max(1);
                    output.push(line);
                    continue;
                }
                self.active = None;
            }
            if self.rule {
                self.rule = false;
                let mut rule = Line::default();
                for (index, width) in self.widths.iter().enumerate() {
                    if index > 0 {
                        rule.push("  ", MUTED);
                    }
                    rule.push(&"─".repeat(*width), MUTED);
                }
                let mut rule = rule.at_source(self.origin);
                rule.origin.as_mut().unwrap().character = self.character;
                output.push(rule);
            }
            break;
        }
        output
    }

    fn row(&self, source: &str) -> Vec<String> {
        let (mut row, _) = cells(source);
        if row.len() > self.headers.len() {
            // Keep surplus model output visible instead of silently discarding it.
            let extra = row.split_off(self.headers.len() - 1);
            row.push(extra.join(" | "));
        }
        row.resize(self.headers.len(), String::new());
        row
    }

    fn fit(&mut self) {
        let available = self
            .columns
            .saturating_sub(self.widths.len().saturating_sub(1) * 2);
        let minimum: Vec<_> = self
            .widths
            .iter()
            .map(|width| (*width).clamp(2, 6))
            .collect();
        if minimum.iter().sum::<usize>() > available {
            self.stacked = true;
            return;
        }
        if self.widths.iter().sum::<usize>() <= available {
            return;
        }
        let natural = self.widths.clone();
        let (mut low, mut high) = (2, *natural.iter().max().unwrap());
        while low < high {
            let cap = low + (high - low).div_ceil(2);
            let total: usize = natural
                .iter()
                .zip(&minimum)
                .map(|(width, min)| (*width).min(cap).max(*min))
                .sum();
            if total <= available {
                low = cap;
            } else {
                high = cap - 1;
            }
        }
        for ((width, natural), min) in self.widths.iter_mut().zip(&natural).zip(minimum) {
            *width = (*natural).min(low).max(min);
        }
        let mut left = available - self.widths.iter().sum::<usize>();
        while left > 0 {
            let Some(index) = natural
                .iter()
                .zip(&self.widths)
                .enumerate()
                .filter(|(_, (natural, width))| natural > width)
                .max_by_key(|(_, (natural, width))| *natural - *width)
                .map(|(index, _)| index)
            else {
                break;
            };
            self.widths[index] += 1;
            left -= 1;
        }
    }

    fn start_row(&self, cells: &[String], style: &'static str, labeled: bool) -> Row {
        if self.stacked {
            Row::Stacked {
                cells: cells
                    .iter()
                    .zip(&self.headers)
                    .map(|(cell, header)| {
                        let mut line = Line::default();
                        if labeled {
                            append(&mut line, &inline::render(header, EMPHASIS));
                            line.push(": ", MUTED);
                        }
                        append(&mut line, &inline::render(cell, style));
                        line.into_wrapped(self.columns, true)
                    })
                    .collect(),
                blank: labeled && self.body_seen,
            }
        } else {
            Row::Columns(
                cells
                    .iter()
                    .zip(&self.widths)
                    .map(|(cell, width)| inline::render(cell, style).into_wrapped(*width, true))
                    .collect(),
            )
        }
    }
}

impl Row {
    fn next(&mut self, widths: &[usize], alignments: &[Alignment]) -> Option<Line> {
        match self {
            Self::Stacked { cells, blank } => {
                if std::mem::take(blank) {
                    return Some(Line::default());
                }
                while let Some(cell) = cells.front_mut() {
                    if let Some(line) = cell.next() {
                        return Some(line);
                    }
                    cells.pop_front();
                }
                None
            }
            Self::Columns(cells) => {
                let mut line = Line::default();
                let mut visible = false;
                let last = cells.len() - 1;
                for (index, column) in cells.iter_mut().enumerate() {
                    if index > 0 {
                        line.push("  ", BODY);
                    }
                    let cell = column.next();
                    visible |= cell.is_some();
                    let cell = cell.unwrap_or_default();
                    let padding = widths[index].saturating_sub(text::cells(&cell.plain()));
                    let left = match alignments[index] {
                        Alignment::Left => 0,
                        Alignment::Right => padding,
                        Alignment::Center => padding / 2,
                    };
                    line.push(&" ".repeat(left), BODY);
                    append(&mut line, &cell);
                    if index < last {
                        line.push(&" ".repeat(padding - left), BODY);
                    }
                }
                visible.then_some(line)
            }
        }
    }
}

pub(super) fn source_line(value: &str, position: usize) -> Option<(&str, usize)> {
    if position >= value.len() {
        return None;
    }
    let end = value[position..]
        .find('\n')
        .map_or(value.len(), |offset| position + offset);
    Some((&value[position..end], end + 1))
}

fn visual_width(value: &str) -> usize {
    text::cells(&inline::render(value, BODY).plain())
}

fn append(line: &mut Line, value: &Line) {
    for span in &value.spans {
        line.push(&span.text, span.style);
    }
}

fn alignment(value: &str) -> Option<Alignment> {
    let value = value.trim();
    let body = value.strip_prefix(':').unwrap_or(value);
    let body = body.strip_suffix(':').unwrap_or(body);
    if body.is_empty() || !body.bytes().all(|byte| byte == b'-') {
        return None;
    }
    Some(match (value.starts_with(':'), value.ends_with(':')) {
        (true, true) => Alignment::Center,
        (false, true) => Alignment::Right,
        _ => Alignment::Left,
    })
}

fn block_start(source: &str) -> bool {
    let source = source.trim_start();
    let heading = source.bytes().take_while(|byte| *byte == b'#').count();
    source.starts_with('>')
        || opening_fence(source).is_some()
        || horizontal_rule(source)
        || list_item(source).is_some()
        || ((1..=6).contains(&heading)
            && source
                .as_bytes()
                .get(heading)
                .is_some_and(u8::is_ascii_whitespace))
}

// Only unescaped pipes separate cells, including in code spans. Preserve other
// escapes for the inline parser; remove the optional outer empty cells.
fn cells(source: &str) -> (Vec<String>, bool) {
    let source = source.trim();
    let mut characters = source.chars().peekable();
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut pipes = false;
    let mut last_pipe = false;
    while let Some(character) = characters.next() {
        last_pipe = false;
        match character {
            '\\' if characters.peek() == Some(&'|') => {
                characters.next();
                cell.push('|');
            }
            '\\' if characters.peek() == Some(&'\\') => {
                characters.next();
                cell.push_str("\\\\");
            }
            '|' => {
                pipes = true;
                cells.push(std::mem::take(&mut cell).trim().into());
                last_pipe = true;
            }
            character => cell.push(character),
        }
    }
    cells.push(cell.trim().into());
    if source.starts_with('|') {
        cells.remove(0);
    }
    if last_pipe {
        cells.pop();
    }
    (cells, pipes)
}

#[cfg(test)]
mod tests;
