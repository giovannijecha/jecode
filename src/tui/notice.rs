//! Feedback above the lower area, in the terminal's left margin.
use super::{
    line::Line,
    state::Kind,
    text,
    theme::{NOTICE_ERROR, NOTICE_INFO, NOTICE_WARNING, Style},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Level {
    Info,
    Warning,
    Error,
}

impl From<Kind> for Level {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Error => Self::Error,
            Kind::Warning => Self::Warning,
            _ => Self::Info,
        }
    }
}

impl Level {
    fn style(self) -> Style {
        match self {
            Self::Info => NOTICE_INFO,
            Self::Warning => NOTICE_WARNING,
            Self::Error => NOTICE_ERROR,
        }
    }
}

pub(super) fn lines(level: Level, value: &str, columns: usize) -> Vec<Line> {
    Line::new(&text::clean(value), level.style())
        .wrap(columns.max(1), true)
        .into_iter()
        .map(|line| line.shortened(columns))
        .collect()
}

#[cfg(test)]
mod tests;
