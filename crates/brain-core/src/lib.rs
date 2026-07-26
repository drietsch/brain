//! brain-core: the constitutional layer of the substrate.
//!
//! Everything above this crate is replaceable; the properties defined here are
//! not. It provides:
//!
//! - stable and content-derived identity ([`ids`])
//! - the canonical encoding that makes content identity deterministic ([`canonical`])
//! - the object model: every kind of node the graph can hold ([`object`])
//!
//! Invariant: semantically identical objects MUST produce identical canonical
//! bytes and therefore identical [`ids::NodeId`]s. Every other component
//! (dedup, replication, evidence caching, signing) depends on this.

pub mod canonical;
pub mod ids;
pub mod object;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    /// Floats are excluded from the canonical encoding: their textual
    /// rendering is not canonical across platforms and serializers.
    #[error("floating-point numbers are not permitted in canonical encoding")]
    Float,
    #[error("malformed node id: {0}")]
    BadId(String),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}
