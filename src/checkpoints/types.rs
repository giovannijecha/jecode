use super::state::State;
use std::collections::BTreeMap;

pub const TTL_MILLIS: u64 = 24 * 60 * 60 * 1_000;
pub type Issue = (String, String);

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub reference: String,
    pub created: u64,
    pub tool: String,
    pub origin: String,
    pub entries: BTreeMap<String, State>,
    pub roots: Vec<String>,
    pub excluded: Vec<String>,
    pub partial: Vec<Issue>,
}

impl Snapshot {
    pub fn expires(&self) -> u64 {
        self.created.saturating_add(TTL_MILLIS)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Capture {
    pub reference: Option<String>,
    pub created: u64,
    pub saved: usize,
    pub partial: Vec<Issue>,
}

#[derive(Clone, Debug)]
pub struct Summary {
    pub reference: String,
    pub created: u64,
    pub tool: String,
    pub saved: usize,
    pub partial: usize,
}
