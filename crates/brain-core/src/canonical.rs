//! Canonical encoding: the discipline everything else stands on.
//!
//! Rules:
//! - JSON value model, UTF-8 bytes.
//! - Object keys sorted bytewise ascending; no duplicate keys.
//! - No insignificant whitespace.
//! - Integers only (i64/u64 range); floats are rejected outright rather than
//!   normalized, because float rendering is where canonical encodings rot.
//! - Strings escaped by serde_json's deterministic minimal escaping.
//!
//! This module is deliberately small and dependency-light: a second,
//! independent implementation should be easy to write to cross-check it.

use crate::ids::NodeId;
use crate::CoreError;
use serde_json::Value;

/// Encode a JSON value into its canonical byte form.
pub fn canonical_bytes(v: &Value) -> Result<Vec<u8>, CoreError> {
    let mut out = Vec::new();
    write_canonical(v, &mut out)?;
    Ok(out)
}

/// Hash a JSON value's canonical form into a content identity.
pub fn hash_value(v: &Value) -> Result<NodeId, CoreError> {
    Ok(hash_bytes(&canonical_bytes(v)?))
}

/// Hash raw bytes into a content identity. Used by the store to verify that
/// persisted bytes still hash to the id they are filed under.
pub fn hash_bytes(bytes: &[u8]) -> NodeId {
    NodeId::from_bytes(*blake3::hash(bytes).as_bytes())
}

fn write_canonical(v: &Value, out: &mut Vec<u8>) -> Result<(), CoreError> {
    match v {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.extend_from_slice(i.to_string().as_bytes());
            } else if let Some(u) = n.as_u64() {
                out.extend_from_slice(u.to_string().as_bytes());
            } else {
                return Err(CoreError::Float);
            }
        }
        Value::String(s) => {
            // serde_json string escaping is deterministic (minimal escapes).
            let escaped = serde_json::to_string(s)?;
            out.extend_from_slice(escaped.as_bytes());
        }
        Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_canonical(item, out)?;
            }
            out.push(b']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push(b'{');
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                let escaped = serde_json::to_string(key)?;
                out.extend_from_slice(escaped.as_bytes());
                out.push(b':');
                write_canonical(&map[*key], out)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_order_does_not_matter() {
        let a: Value = serde_json::from_str(r#"{"b":1,"a":{"y":2,"x":3}}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"a":{"x":3,"y":2},"b":1}"#).unwrap();
        assert_eq!(canonical_bytes(&a).unwrap(), canonical_bytes(&b).unwrap());
        assert_eq!(hash_value(&a).unwrap(), hash_value(&b).unwrap());
    }

    #[test]
    fn different_content_different_hash() {
        assert_ne!(
            hash_value(&json!({"a": 1})).unwrap(),
            hash_value(&json!({"a": 2})).unwrap()
        );
    }

    #[test]
    fn floats_are_rejected() {
        assert!(matches!(
            canonical_bytes(&json!({"x": 1.5})),
            Err(CoreError::Float)
        ));
    }

    #[test]
    fn encoding_is_compact_and_sorted() {
        let v: Value = serde_json::from_str(r#"{"b": [1, 2], "a": "s"}"#).unwrap();
        assert_eq!(
            String::from_utf8(canonical_bytes(&v).unwrap()).unwrap(),
            r#"{"a":"s","b":[1,2]}"#
        );
    }
}
