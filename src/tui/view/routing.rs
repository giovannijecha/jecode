//! Routing endpoints as a table: price, speed and latency columns with the
//! best measured value of each in green. Narrow panels drop latency, then
//! speed, then price.

use super::panel;
use crate::{
    backend::{Entry, Metrics, price},
    tui::{
        line::Line,
        theme::{DIM, GOOD, MUTED, STRONG},
    },
};

const NAME: usize = 12;
const SPEED: usize = 6;
const LATENCY: usize = 7;
const PRICE_HEADING: &str = "$/M in/out";

pub(super) struct Columns {
    name: usize,
    /// Price column width, or zero without one.
    price: usize,
    speed: bool,
    latency: bool,
    best: Metrics,
}

fn prices(metrics: &Metrics) -> String {
    format!("{}/{}", price(metrics.input), price(metrics.output))
}

fn right(value: &str, width: usize) -> String {
    format!("{value:>width$}")
}

/// The best value among at least two measured endpoints.
fn best(values: impl Iterator<Item = f64>, lowest: bool) -> Option<f64> {
    let values: Vec<f64> = values.collect();
    if values.len() < 2 {
        return None;
    }
    values
        .into_iter()
        .reduce(|a, b| if (b < a) == lowest { b } else { a })
}

impl Columns {
    pub fn new(entries: &[Entry], width: usize) -> Self {
        let measured: Vec<&Metrics> = entries
            .iter()
            .filter_map(|entry| entry.metrics.as_ref())
            .collect();
        let price = measured
            .iter()
            .map(|metrics| prices(metrics).chars().count())
            .chain([PRICE_HEADING.len()])
            .max()
            .filter(|_| !measured.is_empty())
            .unwrap_or(0);
        let mut columns = Self {
            name: 0,
            price,
            speed: !measured.is_empty(),
            latency: !measured.is_empty(),
            best: Metrics {
                input: best(measured.iter().filter_map(|metrics| metrics.input), true),
                output: None,
                throughput: best(
                    measured.iter().filter_map(|metrics| metrics.throughput),
                    false,
                ),
                latency: best(measured.iter().filter_map(|metrics| metrics.latency), true),
            },
        };
        loop {
            columns.name =
                width.saturating_sub(columns.metrics() + 2 * usize::from(columns.metrics() > 0));
            if columns.name >= NAME || columns.metrics() == 0 {
                return columns;
            }
            if columns.latency {
                columns.latency = false;
            } else if columns.speed {
                columns.speed = false;
            } else {
                columns.price = 0;
            }
        }
    }
    /// Whether any figure column fits.
    pub fn measured(&self) -> bool {
        self.metrics() > 0
    }
    fn metrics(&self) -> usize {
        [
            self.price,
            if self.speed { SPEED } else { 0 },
            if self.latency { LATENCY } else { 0 },
        ]
        .into_iter()
        .filter(|width| *width > 0)
        .fold(
            0,
            |sum, width| if sum == 0 { width } else { sum + 2 + width },
        )
    }
    fn cells(&self, cells: [(String, bool); 3]) -> Line {
        let mut line = Line::default();
        let widths = [
            self.price,
            if self.speed { SPEED } else { 0 },
            if self.latency { LATENCY } else { 0 },
        ];
        for (width, (value, good)) in widths.into_iter().zip(cells) {
            if width == 0 {
                continue;
            }
            if !line.spans.is_empty() {
                line.push("  ", MUTED);
            }
            line.push(&right(&value, width), if good { GOOD } else { STRONG });
        }
        line
    }
    pub fn header(&self) -> Line {
        let mut line = panel::fit(Line::new("Endpoint", DIM), self.name, None);
        if self.metrics() > 0 {
            line.push("  ", DIM);
            let mut cells = self.cells([
                (PRICE_HEADING.into(), false),
                ("tok/s".into(), false),
                ("latency".into(), false),
            ]);
            for span in &mut cells.spans {
                span.style = DIM;
            }
            line = line.append(cells);
        }
        line
    }
    /// One endpoint row without the focus marker, `width` cells wide.
    pub fn row(&self, entry: &Entry, width: usize) -> Line {
        let style = if entry.problem.is_some() {
            MUTED
        } else {
            STRONG
        };
        let name = Line::new(&entry.name, style);
        let Some(metrics) = &entry.metrics else {
            // Auto and unlisted choices state their nature where the figures go.
            return panel::spread(name, Line::new(&entry.facts, MUTED), width, None);
        };
        let mut line = panel::fit(name, self.name, None);
        if self.metrics() == 0 {
            return line;
        }
        line.push("  ", MUTED);
        let is = |value: Option<f64>, best: Option<f64>| value.is_some() && value == best;
        let figure =
            |value: Option<f64>, format: &dyn Fn(f64) -> String| value.map_or("?".into(), format);
        line.append(self.cells([
            (prices(metrics), is(metrics.input, self.best.input)),
            (
                figure(metrics.throughput, &|value| format!("{}", value.round())),
                is(metrics.throughput, self.best.throughput),
            ),
            (
                figure(metrics.latency, &|value| format!("{value:.1} s")),
                is(metrics.latency, self.best.latency),
            ),
        ]))
    }
}
