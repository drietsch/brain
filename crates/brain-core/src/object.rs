//! The object model: every kind of node the graph can hold.
//!
//! "The software is in the graph — no files at all" cashes out here: code,
//! specs, evidence, capabilities, intents, receipts, entities, observations
//! and namespaces are all just [`Object`] variants stored in one
//! content-addressed graph. Edges are the `NodeId`/`StableId` references
//! inside objects.
//!
//! Native mode stores `Code`; reflective mode (the twin of external software)
//! stores `Entity` + `Observation`. Both share the same identity scheme, so a
//! twinned entity can later gain a native implementation without re-modeling.

use crate::canonical;
use crate::ids::{NodeId, StableId};
use crate::CoreError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A node in the graph. Immutable once stored; identity = hash of canonical form.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Object {
    /// Native-mode program: a term of the core calculus.
    Code { term: Term },

    /// What an implementation is supposed to do. Types are opaque strings in
    /// the scaffold; a real type system replaces them without changing identity.
    Spec {
        name: String,
        input: String,
        output: String,
        effects: Vec<String>,
        properties: Vec<String>,
    },

    /// A verification claim about another node, tagged with the level that
    /// actually supports it. Authority must never exceed this level.
    Evidence {
        subject: NodeId,
        level: VerificationLevel,
        method: String,
        passed: bool,
        detail: String,
    },

    /// Scoped authority to produce an effect. Possessing a transformation is
    /// never permission to run it; a capability is.
    Capability {
        id: StableId,
        effect: String,
        scope: BTreeMap<String, String>,
    },

    /// Stable conceptual identity for anything: a machine, a service, a
    /// source file in twinned external software, an agent, a person.
    Entity {
        id: StableId,
        entity_kind: String,
        labels: BTreeMap<String, String>,
    },

    /// A time-bound, sourced statement about the world. Observations expire
    /// into staleness; they never silently become false — or eternally true.
    Observation {
        subject: StableId,
        property: String,
        value: String,
        source: String,
        observed_at_ms: u64,
    },

    /// Durable record of intent, written BEFORE a consequential effect is
    /// attempted. The recovery protocol depends on this ordering.
    Intent {
        action: String,
        arg_hash: NodeId,
        capability: Option<String>,
        at_ms: u64,
    },

    /// Durable record of an effect's outcome, written after the attempt.
    Receipt {
        intent: NodeId,
        ok: bool,
        detail: String,
        at_ms: u64,
    },

    /// A view of the graph as a named codebase: name -> node bindings.
    /// Namespaces are themselves content-addressed and carry lineage via
    /// `parent`, so "version control" is just this chain.
    Namespace {
        entries: BTreeMap<String, NodeId>,
        parent: Option<NodeId>,
    },
}

/// The verification taxonomy. Ordered weakest-to-strongest is deliberately NOT
/// implied by declaration order; levels answer different questions and a
/// claim must state which level supports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationLevel {
    Unverified,
    Structural,
    Authorization,
    Transactional,
    Behavioral,
    Empirical,
    Interpretive,
    Formal,
}

/// The core calculus. Deliberately tiny (12 operations); resist enrichment
/// until authoring pain demands it. Effects happen only through `foreign`
/// symbols whose capability requirements live in the runtime registry, so the
/// term language itself cannot smuggle ambient authority.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Term {
    Lit {
        value: Literal,
    },
    Var {
        name: String,
    },
    Lam {
        param: String,
        body: Box<Term>,
    },
    App {
        func: Box<Term>,
        arg: Box<Term>,
    },
    Let {
        name: String,
        value: Box<Term>,
        body: Box<Term>,
    },
    Record {
        fields: BTreeMap<String, Term>,
    },
    Field {
        record: Box<Term>,
        field: String,
    },
    Variant {
        tag: String,
        payload: Box<Term>,
    },
    Match {
        scrutinee: Box<Term>,
        arms: BTreeMap<String, Arm>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<Box<Term>>,
    },
    /// Reference to another `Code` object by content hash: the graph's
    /// replacement for imports, linking, and dependency resolution.
    #[serde(rename = "ref")]
    RefNode {
        node: NodeId,
    },
    /// The only gate to the outside world. `symbol` names an entry in the
    /// runtime's foreign registry, which declares effect class and required
    /// capability.
    Foreign {
        symbol: String,
        arg: Box<Term>,
    },
    /// A typed hole: programs are first-class while incomplete. Evaluating a
    /// hole suspends with `Incomplete` instead of being a syntax error.
    Hole {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Arm {
    pub bind: String,
    pub body: Term,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Literal {
    Int { value: i64 },
    Str { value: String },
    Bool { value: bool },
    Unit,
}

/// Content identity of an object: hash of its canonical encoding.
pub fn hash_object(o: &Object) -> Result<NodeId, CoreError> {
    let v = serde_json::to_value(o)?;
    canonical::hash_value(&v)
}

/// Canonical bytes of an object — what the store persists, so that what is
/// on disk is byte-identical to what was hashed.
pub fn object_bytes(o: &Object) -> Result<Vec<u8>, CoreError> {
    let v = serde_json::to_value(o)?;
    canonical::canonical_bytes(&v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(i: i64) -> Term {
        Term::Lit {
            value: Literal::Int { value: i },
        }
    }

    #[test]
    fn identical_objects_have_identical_ids() {
        let a = Object::Code { term: lit(42) };
        let b = Object::Code { term: lit(42) };
        assert_eq!(hash_object(&a).unwrap(), hash_object(&b).unwrap());
        assert_ne!(
            hash_object(&a).unwrap(),
            hash_object(&Object::Code { term: lit(43) }).unwrap()
        );
    }

    #[test]
    fn objects_roundtrip_through_canonical_bytes() {
        let mut fields = BTreeMap::new();
        fields.insert("a".to_string(), lit(1));
        let o = Object::Code {
            term: Term::Foreign {
                symbol: "core/add".to_string(),
                arg: Box::new(Term::Record { fields }),
            },
        };
        let bytes = object_bytes(&o).unwrap();
        let back: Object = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(o, back);
        assert_eq!(hash_object(&o).unwrap(), hash_object(&back).unwrap());
    }

    #[test]
    fn term_json_shape_matches_authoring_schema() {
        // The wire shape agents author against: tagged with "op"/"kind".
        let json = r#"{"kind":"code","term":{"op":"app",
            "func":{"op":"lam","param":"x","body":{"op":"var","name":"x"}},
            "arg":{"op":"lit","value":{"type":"int","value":7}}}}"#;
        let o: Object = serde_json::from_str(json).unwrap();
        assert!(matches!(o, Object::Code { .. }));
    }
}
