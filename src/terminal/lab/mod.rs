//! Owned presentation layer used by the real account TUI and offline preview.
//! The path from typed state to rows to terminal bytes has no effects.
pub mod activity_view;
pub mod block;
pub mod caps;
pub mod command_view;
pub mod composer;
#[cfg(test)]
mod fixture_tests;
pub mod glyph;
pub mod markdown;
pub mod model;
pub mod motion;
#[cfg(test)]
mod performance_tests;
pub mod picker;
pub mod render;
#[cfg(test)]
mod specimen_tests;
pub mod style;
pub mod text;
pub mod tool_view;
pub mod unicode;
pub mod view;
