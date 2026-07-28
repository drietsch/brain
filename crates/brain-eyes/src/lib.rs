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
mod query;
mod say;
mod state;

pub mod dto;

pub use dto::*;
pub use state::{AppState, Config};

/// Serve Eyes until the process is stopped.
pub fn serve(config: Config) -> Result<(), String> {
    http::serve(config)
}

#[cfg(test)]
mod tests;
