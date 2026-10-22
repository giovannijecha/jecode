//! Workspace checkpoints: a private, shared, append-only archive of the
//! workspace scope that lets any session inspect and reverse file changes.
pub mod actions;
mod activity;
mod capture;
pub mod command;
mod files;
mod frame;
mod gc;
mod hash;
mod history;
mod identity;
mod index;
mod inventory;
mod journal;
mod locks;
#[cfg(test)]
mod measurement_tests;
mod mutation;
mod points;
mod policy;
mod quota;
#[cfg(test)]
mod real_model_tests;
mod reconcile;
mod record;
pub mod render;
mod restore;
mod scan;
pub mod session;
pub mod show;
mod store;
mod targeted;
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod watch;
mod workspace;
pub(crate) use workspace::Workspace;
