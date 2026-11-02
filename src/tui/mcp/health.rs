//! One connection state, named once for the list, the detail and their ink.
use super::{Connection, ErrorKind, Ui};
use crate::tui::theme::{BAD, DIM, MUTED, Style, WARNING};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::tui) enum Health {
    Available(usize),
    Suspended,
    Expired,
    Unreachable,
    Checking,
    Unchecked,
}

impl Health {
    pub(super) fn of(connection: &Connection, ui: &Ui) -> Self {
        if !connection.enabled {
            return Self::Suspended;
        }
        match ui.health.get(&connection.id) {
            Some(Ok(catalog)) => Self::Available(catalog.tools.len()),
            Some(Err(ErrorKind::Authentication)) => Self::Expired,
            Some(Err(_)) => Self::Unreachable,
            None if ui.checks.pending(&connection.id) => Self::Checking,
            None => Self::Unchecked,
        }
    }

    /// The short fact beside a connection in the list.
    pub(in crate::tui) fn fact(self) -> String {
        match self {
            Self::Available(tools) => format!("{tools} tools"),
            Self::Suspended => "suspended".into(),
            Self::Expired => "sign in again".into(),
            Self::Unreachable => "unreachable".into(),
            Self::Checking => "checking…".into(),
            Self::Unchecked => "not checked".into(),
        }
    }

    /// The state line heading a connection's detail.
    pub(in crate::tui) fn state(self) -> String {
        match self {
            Self::Available(tools) => format!("Connected · {tools} tools"),
            Self::Suspended => "Suspended".into(),
            Self::Expired => "Sign-in expired".into(),
            Self::Unreachable => "Not reachable".into(),
            Self::Checking => "Checking…".into(),
            Self::Unchecked => "Not checked".into(),
        }
    }

    pub(in crate::tui) fn style(self) -> Style {
        match self {
            Self::Suspended => DIM,
            Self::Expired => WARNING,
            Self::Unreachable => BAD,
            Self::Available(_) | Self::Checking | Self::Unchecked => MUTED,
        }
    }
}
