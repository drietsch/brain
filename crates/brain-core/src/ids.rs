//! Identity comes in two forms, and the distinction is load-bearing:
//!
//! - [`NodeId`] — content identity. "Is this the exact version that was
//!   observed, verified or executed?" Derived by hashing the canonical
//!   encoding; immutable by construction.
//! - [`StableId`] — conceptual identity. "Is this the same entity across
//!   change?" Survives version changes; names and locations hang off it as
//!   mutable metadata.

use crate::CoreError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// Content identity: BLAKE3 hash of an object's canonical encoding.
/// Rendered as `b3:<64 hex chars>`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub [u8; 32]);

impl NodeId {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        NodeId(bytes)
    }

    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for b in self.0.iter() {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }

    pub fn parse(s: &str) -> Result<Self, CoreError> {
        let hex = s
            .strip_prefix("b3:")
            .ok_or_else(|| CoreError::BadId(s.to_string()))?;
        if hex.len() != 64 {
            return Err(CoreError::BadId(s.to_string()));
        }
        let mut out = [0u8; 32];
        for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
            let hi = hex_val(chunk[0]).ok_or_else(|| CoreError::BadId(s.to_string()))?;
            let lo = hex_val(chunk[1]).ok_or_else(|| CoreError::BadId(s.to_string()))?;
            out[i] = (hi << 4) | lo;
        }
        Ok(NodeId(out))
    }
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "b3:{}", self.to_hex())
    }
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "b3:{}", &self.to_hex()[..12])
    }
}

impl Serialize for NodeId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for NodeId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        NodeId::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// Conceptual identity: a stable handle that survives version change.
/// Rendered as `sid:<32 hex chars>` when derived, but any opaque string is
/// accepted so external identity schemes can be carried through.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct StableId(pub String);

impl StableId {
    /// Derive a stable id deterministically from a list of naming parts.
    /// Deterministic derivation keeps reflective-mode ingestion idempotent:
    /// observing the same external entity twice yields the same identity.
    pub fn derive(parts: &[&str]) -> Self {
        let mut hasher = blake3::Hasher::new();
        for p in parts {
            hasher.update(&(p.len() as u64).to_le_bytes());
            hasher.update(p.as_bytes());
        }
        let hash = hasher.finalize();
        let hex = hash.to_hex();
        StableId(format!("sid:{}", &hex.as_str()[..32]))
    }
}

impl fmt::Display for StableId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_id_roundtrip() {
        let id = NodeId::from_bytes([7u8; 32]);
        let parsed = NodeId::parse(&id.to_string()).unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn node_id_rejects_malformed() {
        assert!(NodeId::parse("nope").is_err());
        assert!(NodeId::parse("b3:1234").is_err());
    }

    #[test]
    fn stable_id_is_deterministic() {
        assert_eq!(
            StableId::derive(&["file", "src/lib.rs"]),
            StableId::derive(&["file", "src/lib.rs"])
        );
        assert_ne!(
            StableId::derive(&["file", "src/lib.rs"]),
            StableId::derive(&["file", "src/main.rs"])
        );
        // Length-prefixing prevents concatenation collisions.
        assert_ne!(
            StableId::derive(&["ab", "c"]),
            StableId::derive(&["a", "bc"])
        );
    }
}
