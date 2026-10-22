//! Bounded verified object cache for one Store handle.
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

const CACHE_BYTES: usize = 64 << 20;

#[derive(Default)]
pub(super) struct Cache {
    entries: HashMap<u64, Arc<[u8]>>,
    order: VecDeque<u64>,
    bytes: usize,
}

impl Cache {
    pub(super) fn get(&self, id: u64) -> Option<Arc<[u8]>> {
        self.entries.get(&id).cloned()
    }

    pub(super) fn insert(&mut self, id: u64, bytes: Arc<[u8]>) {
        if bytes.len() > CACHE_BYTES / 4 || self.entries.contains_key(&id) {
            return;
        }
        self.bytes += bytes.len();
        self.entries.insert(id, bytes);
        self.order.push_back(id);
        while self.bytes > CACHE_BYTES {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some(bytes) = self.entries.remove(&old) {
                self.bytes -= bytes.len();
            }
        }
    }

    pub(super) fn remove(&mut self, id: u64) {
        if let Some(bytes) = self.entries.remove(&id) {
            self.bytes -= bytes.len();
            self.order.retain(|entry| *entry != id);
        }
    }
}
