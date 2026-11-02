//! Owned, provider-independent MCP connections shared by tools and the TUI.

pub mod auth;
mod client;
pub mod config;
mod runtime;
pub mod store;
pub mod transport;
mod types;
mod wire;

pub use client::Client;
pub use runtime::Runtime;
pub use store::Store;
pub use types::{AuthMethod, Catalog, Connection, Error, ErrorKind, Tool};

mod bridge;
#[cfg(test)]
pub use bridge::execute;
pub use bridge::{
    connection_name, execute_with, include_credentials, protected_label, summary, without_query,
};
