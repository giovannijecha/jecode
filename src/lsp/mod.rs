//! User-selected language servers. Configuration is global; sessions belong to a Tools instance.
mod client;
mod config;
mod documents;
mod executable;
#[cfg(test)]
pub(crate) mod fixture;
mod glob;
mod locations;
mod runtime;
mod store;
mod transport;

#[cfg(test)]
mod tests;

pub use config::{CheckFailure, CheckObservation, Profile};
pub use runtime::{Outcome, Request, Runtime, Status, check};
pub use store::Store;
