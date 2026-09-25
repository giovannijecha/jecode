//! An owned coding harness: transport, sessions, tools and inline terminal presentation.

pub mod command;
pub mod http;
pub(crate) mod image;
pub mod json;
pub mod providers;
pub mod session;
pub mod state;
pub mod stream;
pub mod terminal;
pub mod tls;
pub mod tools;
pub mod workspace;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
#[path = "../tests/support/workspace.rs"]
pub(crate) mod workspace_fixture;
