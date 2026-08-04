//! Eyes: the visual layer over everything the brain knows.
//!
//! Eyes reads two things and only two things: **judgments** (what the graph
//! concludes, and why) and **content** (the actual documents, decisions,
//! plans and test results). It does not browse structure — a picture of a
//! thousand nodes says only "it is complicated" (ADR-024).
//!
//! The store stays the system of record and cortex stays the disposable
//! query layer. This crate owns no durable state, writes nothing, and every
//! response names the graph snapshot it was computed from. Human wording
//! lives server-side in [`say`] so there is exactly one voice and the
//! browser never invents a status model (ADR-023).

mod body;
mod http;
mod say;
mod state;

pub mod dto;
/// The projection layer, shared with the other read-only surfaces.
///
/// A WebDAV mount renders the same shelves this crate renders for the
/// browser, through these same functions — so a decision read from a
/// mounted file and the same decision read in the cockpit carry the
/// identical sentences, composed once in [`say`].
pub mod query;

pub use dto::*;
pub use state::{AppState, Config, Loaded};

/// Serve Eyes until the process is stopped.
pub fn serve(config: Config) -> Result<(), String> {
    http::serve(config)
}

#[cfg(test)]
mod tests;
