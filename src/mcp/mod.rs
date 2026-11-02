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
pub use bridge::{execute, execute_with, include_credentials, label, summary};
