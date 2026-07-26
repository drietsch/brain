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

    /// An observed, time-bound, typed edge between two entities. Observations
    /// state facts *about* one entity; Relations state structure *between*
    /// entities — the twin's glue (a file `contains` a symbol, a file
    /// `imports` a module). Like observations, they are claims at a moment,
    /// not eternal truths.
    Relation {
        from: StableId,
        predicate: String,
        to: StableId,
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

/// Alpha-normalization: rename every binder (and its bound occurrences) to a
/// canonical de Bruijn-level name (`_0`, `_1`, ... by binding depth), so that
/// alpha-equivalent terms have identical canonical form and therefore
/// identical content identity. "Identity before names": binder names are
/// projection-level, and must not leak into hashes.
///
/// Free variables are left untouched (top-level Code is closed; a free
/// variable literally named `_N` in an open term could be captured — a
/// documented pathological edge, not reachable from closed programs).
pub fn alpha_normalize(term: &Term) -> Term {
    norm(term, &mut Vec::new())
}

fn canon_name(depth: usize) -> String {
    format!("_{depth}")
}

fn norm(t: &Term, scope: &mut Vec<(String, String)>) -> Term {
    match t {
        Term::Lit { .. } | Term::RefNode { .. } | Term::Hole { .. } => t.clone(),
        Term::Var { name } => Term::Var {
            name: scope
                .iter()
                .rev()
                .find(|(n, _)| n == name)
                .map(|(_, c)| c.clone())
                .unwrap_or_else(|| name.clone()),
        },
        Term::Lam { param, body } => {
            let canon = canon_name(scope.len());
            scope.push((param.clone(), canon.clone()));
            let body = Box::new(norm(body, scope));
            scope.pop();
            Term::Lam { param: canon, body }
        }
        Term::Let { name, value, body } => {
            let value = Box::new(norm(value, scope));
            let canon = canon_name(scope.len());
            scope.push((name.clone(), canon.clone()));
            let body = Box::new(norm(body, scope));
            scope.pop();
            Term::Let { name: canon, value, body }
        }
        Term::App { func, arg } => Term::App {
            func: Box::new(norm(func, scope)),
            arg: Box::new(norm(arg, scope)),
        },
        Term::Record { fields } => Term::Record {
            fields: fields.iter().map(|(k, v)| (k.clone(), norm(v, scope))).collect(),
        },
        Term::Field { record, field } => Term::Field {
            record: Box::new(norm(record, scope)),
            field: field.clone(),
        },
        Term::Variant { tag, payload } => Term::Variant {
            tag: tag.clone(),
            payload: Box::new(norm(payload, scope)),
        },
        Term::Match { scrutinee, arms, default } => Term::Match {
            scrutinee: Box::new(norm(scrutinee, scope)),
            arms: arms
                .iter()
                .map(|(tag, arm)| {
                    let canon = canon_name(scope.len());
                    scope.push((arm.bind.clone(), canon.clone()));
                    let body = norm(&arm.body, scope);
                    scope.pop();
                    (tag.clone(), Arm { bind: canon, body })
                })
                .collect(),
            default: default.as_ref().map(|d| Box::new(norm(d, scope))),
        },
        Term::Foreign { symbol, arg } => Term::Foreign {
            symbol: symbol.clone(),
            arg: Box::new(norm(arg, scope)),
        },
    }
}

/// The form of an object as stored and hashed: Code terms are
/// alpha-normalized. The store applies this at the put boundary, so what is
/// on disk is always the canonical form and stored bytes re-hash to the id.
pub fn canonicalize(o: &Object) -> Object {
    match o {
        Object::Code { term } => Object::Code { term: alpha_normalize(term) },
        other => other.clone(),
    }
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
    fn relations_roundtrip_and_hash_stably() {
        let r = Object::Relation {
            from: StableId::derive(&["file", "src/lib.rs"]),
            predicate: "contains".to_string(),
            to: StableId::derive(&["symbol", "src/lib.rs", "fn", "main"]),
            source: "twin".to_string(),
            observed_at_ms: 42,
        };
        let bytes = object_bytes(&r).unwrap();
        let back: Object = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(r, back);
        assert_eq!(hash_object(&r).unwrap(), hash_object(&back).unwrap());
        // canonicalize is a passthrough for non-Code objects.
        assert_eq!(canonicalize(&r), r);
    }

    #[test]
    fn alpha_equivalent_terms_share_canonical_identity() {
        let f = |p: &str| Object::Code {
            term: Term::Lam {
                param: p.to_string(),
                body: Box::new(Term::Var { name: p.to_string() }),
            },
        };
        assert_ne!(
            hash_object(&f("x")).unwrap(),
            hash_object(&f("y")).unwrap(),
            "raw hashes differ by binder name"
        );
        assert_eq!(
            hash_object(&canonicalize(&f("x"))).unwrap(),
            hash_object(&canonicalize(&f("y"))).unwrap(),
            "canonical hashes must not"
        );
    }

    #[test]
    fn alpha_normalization_handles_shadowing_and_free_vars() {
        // \x -> (\x -> x) x   — inner x binds to inner lam, outer to outer.
        let t = Term::Lam {
            param: "x".to_string(),
            body: Box::new(Term::App {
                func: Box::new(Term::Lam {
                    param: "x".to_string(),
                    body: Box::new(Term::Var { name: "x".to_string() }),
                }),
                arg: Box::new(Term::Var { name: "x".to_string() }),
            }),
        };
        let n = alpha_normalize(&t);
        let expected = Term::Lam {
            param: "_0".to_string(),
            body: Box::new(Term::App {
                func: Box::new(Term::Lam {
                    param: "_1".to_string(),
                    body: Box::new(Term::Var { name: "_1".to_string() }),
                }),
                arg: Box::new(Term::Var { name: "_0".to_string() }),
            }),
        };
        assert_eq!(n, expected);

        // Free variables pass through untouched.
        let free = Term::Var { name: "unbound".to_string() };
        assert_eq!(alpha_normalize(&free), free);
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
