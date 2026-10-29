//! A record with a revision that every write advances, so an observer copies
//! only the records that changed since it last read them.

use std::{
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicU64, Ordering},
};

/// Unique in the process, so records of a replaced collaboration never match.
static NEXT: AtomicU64 = AtomicU64::new(1);

fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

pub(super) struct Tracked<T> {
    value: T,
    revision: u64,
}

impl<T> Tracked<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            revision: next(),
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn into_inner(self) -> T {
        self.value
    }
}

impl<T> Deref for Tracked<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> DerefMut for Tracked<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.revision = next();
        &mut self.value
    }
}
