//! brain-runtime: the interpreter IS the fabric.
//!
//! A fuel-metered tree walker over the core calculus with three structural
//! guarantees:
//!
//! - **No ambient authority.** The term language has no I/O. The only gate to
//!   the outside world is a `foreign` symbol, and every effectful symbol
//!   declares the capability it requires; the interpreter refuses the call if
//!   the evaluation context has not been granted it.
//! - **Effects only through the boundary.** Every external foreign call is
//!   wrapped in `EffectPort::begin` (durable intent, before) and
//!   `EffectPort::commit` (receipt, after). The interpreter cannot reach an
//!   external effect any other way.
//! - **Holes suspend, they don't crash.** Evaluating a hole returns
//!   `Incomplete`, so partial programs are runnable objects, not errors.
//!
//! The runtime depends only on brain-core; storage arrives through the
//! `CodeSource` and `EffectPort` traits so it can be driven by the real store
//! or by in-memory test doubles.

use brain_core::ids::NodeId;
use brain_core::object::{Arm, Literal, Term};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

pub type Env = BTreeMap<String, Value>;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Str(String),
    Bool(bool),
    Unit,
    Record(BTreeMap<String, Value>),
    Variant { tag: String, payload: Box<Value> },
    Closure { param: String, body: Term, env: Env },
}

/// Render a value as JSON — used for receipts, arg hashing at the effect
/// boundary, and CLI display. Closures render opaquely: code identity belongs
/// to the graph, not to a value dump.
pub fn value_to_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Int(i) => json!(i),
        Value::Str(s) => json!(s),
        Value::Bool(b) => json!(b),
        Value::Unit => serde_json::Value::Null,
        Value::Record(fields) => {
            let mut m = serde_json::Map::new();
            for (k, val) in fields {
                m.insert(k.clone(), value_to_json(val));
            }
            serde_json::Value::Object(m)
        }
        Value::Variant { tag, payload } => {
            json!({ "variant": tag, "payload": value_to_json(payload) })
        }
        Value::Closure { param, .. } => json!({ "closure": param }),
    }
}

// ---------------------------------------------------------------------------
// Boundary traits
// ---------------------------------------------------------------------------

/// Where `ref` nodes resolve their target code from.
pub trait CodeSource {
    fn code(&self, id: &NodeId) -> Result<Term, String>;
}

/// A code source with nothing in it; refs always fail. Useful for closed terms.
pub struct NoCode;
impl CodeSource for NoCode {
    fn code(&self, id: &NodeId) -> Result<Term, String> {
        Err(format!("no code source available for {id}"))
    }
}

/// The effect boundary. `begin` must durably record the intent BEFORE the
/// effect runs and return an intent token; `commit` records the receipt after.
pub trait EffectPort {
    fn begin(&mut self, symbol: &str, arg: &Value) -> Result<String, String>;
    fn commit(&mut self, token: &str, result: &Result<Value, String>);
}

/// Effect port that refuses everything: for contexts where no external effect
/// is permitted at all (simulation branches, pure evaluation).
pub struct DenyEffects;
impl EffectPort for DenyEffects {
    fn begin(&mut self, symbol: &str, _arg: &Value) -> Result<String, String> {
        Err(format!("external effects are not permitted here: {symbol}"))
    }
    fn commit(&mut self, _token: &str, _result: &Result<Value, String>) {}
}

/// In-memory recording effect port for tests and dry runs.
#[derive(Default)]
pub struct MemEffects {
    pub begun: Vec<(String, serde_json::Value)>,
    pub committed: Vec<(String, bool)>,
}
impl EffectPort for MemEffects {
    fn begin(&mut self, symbol: &str, arg: &Value) -> Result<String, String> {
        let token = format!("mem-{}", self.begun.len());
        self.begun.push((symbol.to_string(), value_to_json(arg)));
        Ok(token)
    }
    fn commit(&mut self, token: &str, result: &Result<Value, String>) {
        self.committed.push((token.to_string(), result.is_ok()));
    }
}

// ---------------------------------------------------------------------------
// Foreign registry
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectClass {
    /// No externally visible effect; runs without touching the boundary.
    Pure,
    /// Externally visible effect; must pass capability check and boundary.
    External,
}

pub struct ForeignFn {
    pub effect: EffectClass,
    /// Capability name required to invoke this symbol; `None` only for Pure.
    pub requires: Option<String>,
    pub run: fn(&Value) -> Result<Value, String>,
}

#[derive(Default)]
pub struct Registry {
    map: BTreeMap<String, ForeignFn>,
}

impl Registry {
    pub fn register(&mut self, symbol: &str, f: ForeignFn) {
        self.map.insert(symbol.to_string(), f);
    }

    pub fn get(&self, symbol: &str) -> Option<&ForeignFn> {
        self.map.get(symbol)
    }

    /// The minimal builtin set. Pure helpers plus one deliberately trivial
    /// external effect (`io/echo`) so the whole intent/receipt path can be
    /// exercised end to end.
    pub fn with_builtins() -> Registry {
        let mut r = Registry::default();
        r.register(
            "core/add",
            ForeignFn {
                effect: EffectClass::Pure,
                requires: None,
                run: builtin_add,
            },
        );
        r.register(
            "core/sub",
            ForeignFn {
                effect: EffectClass::Pure,
                requires: None,
                run: builtin_sub,
            },
        );
        r.register(
            "core/mul",
            ForeignFn {
                effect: EffectClass::Pure,
                requires: None,
                run: builtin_mul,
            },
        );
        r.register(
            "core/lt",
            ForeignFn {
                effect: EffectClass::Pure,
                requires: None,
                run: builtin_lt,
            },
        );
        r.register(
            "core/if",
            ForeignFn {
                effect: EffectClass::Pure,
                requires: None,
                run: builtin_if,
            },
        );
        r.register(
            "core/concat",
            ForeignFn {
                effect: EffectClass::Pure,
                requires: None,
                run: builtin_concat,
            },
        );
        r.register(
            "core/eq",
            ForeignFn {
                effect: EffectClass::Pure,
                requires: None,
                run: builtin_eq,
            },
        );
        r.register(
            "io/echo",
            ForeignFn {
                effect: EffectClass::External,
                requires: Some("io".to_string()),
                run: |arg| Ok(arg.clone()),
            },
        );
        r
    }
}

fn two_fields<'a>(arg: &'a Value, sym: &str) -> Result<(&'a Value, &'a Value), String> {
    if let Value::Record(fields) = arg {
        match (fields.get("a"), fields.get("b")) {
            (Some(a), Some(b)) => return Ok((a, b)),
            _ => {}
        }
    }
    Err(format!("{sym} expects a record with fields a and b"))
}

fn builtin_add(arg: &Value) -> Result<Value, String> {
    match two_fields(arg, "core/add")? {
        (Value::Int(a), Value::Int(b)) => a
            .checked_add(*b)
            .map(Value::Int)
            .ok_or_else(|| "core/add: integer overflow".to_string()),
        _ => Err("core/add expects integer fields".to_string()),
    }
}

fn builtin_sub(arg: &Value) -> Result<Value, String> {
    match two_fields(arg, "core/sub")? {
        (Value::Int(a), Value::Int(b)) => a
            .checked_sub(*b)
            .map(Value::Int)
            .ok_or_else(|| "core/sub: integer overflow".to_string()),
        _ => Err("core/sub expects integer fields".to_string()),
    }
}

fn builtin_mul(arg: &Value) -> Result<Value, String> {
    match two_fields(arg, "core/mul")? {
        (Value::Int(a), Value::Int(b)) => a
            .checked_mul(*b)
            .map(Value::Int)
            .ok_or_else(|| "core/mul: integer overflow".to_string()),
        _ => Err("core/mul expects integer fields".to_string()),
    }
}

fn builtin_lt(arg: &Value) -> Result<Value, String> {
    match two_fields(arg, "core/lt")? {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a < b)),
        _ => Err("core/lt expects integer fields".to_string()),
    }
}

/// Eager conditional: both branches are already evaluated (record fields).
/// Acceptable for pure scaffold tasks; a lazy `if` op in the calculus is
/// future work, gated on authoring evidence.
fn builtin_if(arg: &Value) -> Result<Value, String> {
    if let Value::Record(fields) = arg {
        match (fields.get("cond"), fields.get("then"), fields.get("else")) {
            (Some(Value::Bool(c)), Some(t), Some(e)) => {
                return Ok(if *c { t.clone() } else { e.clone() });
            }
            _ => {}
        }
    }
    Err("core/if expects a record with fields cond (bool), then, else".to_string())
}

fn builtin_concat(arg: &Value) -> Result<Value, String> {
    match two_fields(arg, "core/concat")? {
        (Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{a}{b}"))),
        _ => Err("core/concat expects string fields".to_string()),
    }
}

fn builtin_eq(arg: &Value) -> Result<Value, String> {
    let (a, b) = two_fields(arg, "core/eq")?;
    Ok(Value::Bool(a == b))
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

#[derive(Debug, Error, PartialEq)]
pub enum EvalError {
    #[error("fuel exhausted")]
    FuelExhausted,
    #[error("incomplete: hole '{hole}' reached")]
    Incomplete { hole: String },
    #[error("unbound variable '{0}'")]
    Unbound(String),
    #[error("type error: {0}")]
    TypeError(String),
    #[error("unknown foreign symbol '{0}'")]
    UnknownForeign(String),
    #[error("capability denied: '{symbol}' requires capability '{capability}'")]
    CapabilityDenied { symbol: String, capability: String },
    #[error("effect boundary refused: {0}")]
    BoundaryRefused(String),
    #[error("effect failed: {0}")]
    EffectFailed(String),
    #[error("bad ref: {0}")]
    BadRef(String),
}

pub struct EvalCtx<'a> {
    pub fuel: u64,
    /// Capabilities granted to this evaluation. Empty set = pure-only.
    pub caps: BTreeSet<String>,
    pub registry: &'a Registry,
    pub code: &'a dyn CodeSource,
    pub effects: &'a mut dyn EffectPort,
}

impl<'a> EvalCtx<'a> {
    fn spend(&mut self) -> Result<(), EvalError> {
        if self.fuel == 0 {
            return Err(EvalError::FuelExhausted);
        }
        self.fuel -= 1;
        Ok(())
    }
}

/// Evaluate a closed term (empty environment).
pub fn eval_closed(ctx: &mut EvalCtx, term: &Term) -> Result<Value, EvalError> {
    eval(ctx, &Env::new(), term)
}

pub fn eval(ctx: &mut EvalCtx, env: &Env, term: &Term) -> Result<Value, EvalError> {
    ctx.spend()?;
    match term {
        Term::Lit { value } => Ok(match value {
            Literal::Int { value } => Value::Int(*value),
            Literal::Str { value } => Value::Str(value.clone()),
            Literal::Bool { value } => Value::Bool(*value),
            Literal::Unit => Value::Unit,
        }),

        Term::Var { name } => env
            .get(name)
            .cloned()
            .ok_or_else(|| EvalError::Unbound(name.clone())),

        Term::Lam { param, body } => Ok(Value::Closure {
            param: param.clone(),
            body: (**body).clone(),
            env: env.clone(),
        }),

        Term::App { func, arg } => {
            let f = eval(ctx, env, func)?;
            let a = eval(ctx, env, arg)?;
            match f {
                Value::Closure {
                    param,
                    body,
                    env: closure_env,
                } => {
                    let mut inner = closure_env;
                    inner.insert(param, a);
                    eval(ctx, &inner, &body)
                }
                other => Err(EvalError::TypeError(format!(
                    "cannot apply non-function value {other:?}"
                ))),
            }
        }

        Term::Let { name, value, body } => {
            let v = eval(ctx, env, value)?;
            let mut inner = env.clone();
            inner.insert(name.clone(), v);
            eval(ctx, &inner, body)
        }

        Term::Record { fields } => {
            let mut out = BTreeMap::new();
            for (k, t) in fields {
                out.insert(k.clone(), eval(ctx, env, t)?);
            }
            Ok(Value::Record(out))
        }

        Term::Field { record, field } => match eval(ctx, env, record)? {
            Value::Record(fields) => fields
                .get(field)
                .cloned()
                .ok_or_else(|| EvalError::TypeError(format!("record has no field '{field}'"))),
            other => Err(EvalError::TypeError(format!(
                "field access on non-record value {other:?}"
            ))),
        },

        Term::Variant { tag, payload } => Ok(Value::Variant {
            tag: tag.clone(),
            payload: Box::new(eval(ctx, env, payload)?),
        }),

        Term::Match {
            scrutinee,
            arms,
            default,
        } => match eval(ctx, env, scrutinee)? {
            Value::Variant { tag, payload } => {
                if let Some(Arm { bind, body }) = arms.get(&tag) {
                    let mut inner = env.clone();
                    inner.insert(bind.clone(), *payload);
                    eval(ctx, &inner, body)
                } else if let Some(d) = default {
                    eval(ctx, env, d)
                } else {
                    Err(EvalError::TypeError(format!("unhandled variant '{tag}'")))
                }
            }
            other => Err(EvalError::TypeError(format!(
                "match on non-variant value {other:?}"
            ))),
        },

        Term::RefNode { node } => {
            // Top-level definitions are closed: refs evaluate in an empty
            // environment, never capturing the caller's scope.
            let target = ctx.code.code(node).map_err(EvalError::BadRef)?;
            eval(ctx, &Env::new(), &target)
        }

        Term::Foreign { symbol, arg } => {
            let argv = eval(ctx, env, arg)?;
            let f = ctx
                .registry
                .get(symbol)
                .ok_or_else(|| EvalError::UnknownForeign(symbol.clone()))?;
            if let Some(cap) = &f.requires {
                if !ctx.caps.contains(cap) {
                    return Err(EvalError::CapabilityDenied {
                        symbol: symbol.clone(),
                        capability: cap.clone(),
                    });
                }
            }
            match f.effect {
                EffectClass::Pure => (f.run)(&argv).map_err(EvalError::EffectFailed),
                EffectClass::External => {
                    // Intent BEFORE the effect; receipt after — always.
                    let token = ctx
                        .effects
                        .begin(symbol, &argv)
                        .map_err(EvalError::BoundaryRefused)?;
                    let result = (f.run)(&argv);
                    ctx.effects.commit(&token, &result);
                    result.map_err(EvalError::EffectFailed)
                }
            }
        }

        Term::Hole { id, .. } => Err(EvalError::Incomplete { hole: id.clone() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(i: i64) -> Term {
        Term::Lit {
            value: Literal::Int { value: i },
        }
    }

    fn ctx<'a>(
        registry: &'a Registry,
        effects: &'a mut dyn EffectPort,
        caps: &[&str],
    ) -> EvalCtx<'a> {
        EvalCtx {
            fuel: 10_000,
            caps: caps.iter().map(|s| s.to_string()).collect(),
            registry,
            code: &NoCode,
            effects,
        }
    }

    fn add(a: Term, b: Term) -> Term {
        let mut fields = BTreeMap::new();
        fields.insert("a".to_string(), a);
        fields.insert("b".to_string(), b);
        Term::Foreign {
            symbol: "core/add".to_string(),
            arg: Box::new(Term::Record { fields }),
        }
    }

    #[test]
    fn lambda_application_and_pure_foreign() {
        // (\x -> add x 2) 40  =>  42
        let term = Term::App {
            func: Box::new(Term::Lam {
                param: "x".to_string(),
                body: Box::new(add(
                    Term::Var {
                        name: "x".to_string(),
                    },
                    int(2),
                )),
            }),
            arg: Box::new(int(40)),
        };
        let registry = Registry::with_builtins();
        let mut effects = MemEffects::default();
        let mut c = ctx(&registry, &mut effects, &[]);
        assert_eq!(eval_closed(&mut c, &term).unwrap(), Value::Int(42));
        // Pure calls never touch the boundary.
        assert!(effects.begun.is_empty());
    }

    #[test]
    fn let_match_and_variants() {
        // let v = some(5) in match v { some x -> add x 1 } default 0
        let mut arms = BTreeMap::new();
        arms.insert(
            "some".to_string(),
            Arm {
                bind: "x".to_string(),
                body: add(
                    Term::Var {
                        name: "x".to_string(),
                    },
                    int(1),
                ),
            },
        );
        let term = Term::Let {
            name: "v".to_string(),
            value: Box::new(Term::Variant {
                tag: "some".to_string(),
                payload: Box::new(int(5)),
            }),
            body: Box::new(Term::Match {
                scrutinee: Box::new(Term::Var {
                    name: "v".to_string(),
                }),
                arms,
                default: Some(Box::new(int(0))),
            }),
        };
        let registry = Registry::with_builtins();
        let mut effects = MemEffects::default();
        let mut c = ctx(&registry, &mut effects, &[]);
        assert_eq!(eval_closed(&mut c, &term).unwrap(), Value::Int(6));
    }

    #[test]
    fn comparison_and_conditional_builtins() {
        // abs(-7) = if (lt -7 0) then (sub 0 -7) else -7  =>  7
        let x = -7;
        let mut lt_fields = BTreeMap::new();
        lt_fields.insert("a".to_string(), int(x));
        lt_fields.insert("b".to_string(), int(0));
        let mut sub_fields = BTreeMap::new();
        sub_fields.insert("a".to_string(), int(0));
        sub_fields.insert("b".to_string(), int(x));
        let mut if_fields = BTreeMap::new();
        if_fields.insert(
            "cond".to_string(),
            Term::Foreign {
                symbol: "core/lt".to_string(),
                arg: Box::new(Term::Record { fields: lt_fields }),
            },
        );
        if_fields.insert(
            "then".to_string(),
            Term::Foreign {
                symbol: "core/sub".to_string(),
                arg: Box::new(Term::Record { fields: sub_fields }),
            },
        );
        if_fields.insert("else".to_string(), int(x));
        let term = Term::Foreign {
            symbol: "core/if".to_string(),
            arg: Box::new(Term::Record { fields: if_fields }),
        };
        let registry = Registry::with_builtins();
        let mut effects = MemEffects::default();
        let mut c = ctx(&registry, &mut effects, &[]);
        assert_eq!(eval_closed(&mut c, &term).unwrap(), Value::Int(7));
    }

    #[test]
    fn holes_suspend_instead_of_crashing() {
        let term = add(
            int(1),
            Term::Hole {
                id: "h0".to_string(),
                expected: Some("int".to_string()),
            },
        );
        let registry = Registry::with_builtins();
        let mut effects = MemEffects::default();
        let mut c = ctx(&registry, &mut effects, &[]);
        assert_eq!(
            eval_closed(&mut c, &term),
            Err(EvalError::Incomplete {
                hole: "h0".to_string()
            })
        );
    }

    #[test]
    fn alpha_normalization_preserves_semantics_under_shadowing() {
        // (\x -> (\x -> add x 1) (add x 2)) 10  =>  (10+2)+1 = 13
        let inner = Term::Lam {
            param: "x".to_string(),
            body: Box::new(add(
                Term::Var {
                    name: "x".to_string(),
                },
                int(1),
            )),
        };
        let outer = Term::Lam {
            param: "x".to_string(),
            body: Box::new(Term::App {
                func: Box::new(inner),
                arg: Box::new(add(
                    Term::Var {
                        name: "x".to_string(),
                    },
                    int(2),
                )),
            }),
        };
        let term = Term::App {
            func: Box::new(outer),
            arg: Box::new(int(10)),
        };
        let normalized = brain_core::object::alpha_normalize(&term);
        assert_ne!(term, normalized, "binder names should have changed");

        let registry = Registry::with_builtins();
        for t in [&term, &normalized] {
            let mut effects = MemEffects::default();
            let mut c = ctx(&registry, &mut effects, &[]);
            assert_eq!(eval_closed(&mut c, t).unwrap(), Value::Int(13));
        }
    }

    #[test]
    fn fuel_exhaustion_halts_evaluation() {
        // omega: (\x -> x x)(\x -> x x) — must halt via fuel, not hang.
        let self_app = Term::Lam {
            param: "x".to_string(),
            body: Box::new(Term::App {
                func: Box::new(Term::Var {
                    name: "x".to_string(),
                }),
                arg: Box::new(Term::Var {
                    name: "x".to_string(),
                }),
            }),
        };
        let omega = Term::App {
            func: Box::new(self_app.clone()),
            arg: Box::new(self_app),
        };
        let registry = Registry::with_builtins();
        let mut effects = MemEffects::default();
        let mut c = ctx(&registry, &mut effects, &[]);
        c.fuel = 500;
        assert_eq!(eval_closed(&mut c, &omega), Err(EvalError::FuelExhausted));
    }

    #[test]
    fn external_effect_requires_capability() {
        let term = Term::Foreign {
            symbol: "io/echo".to_string(),
            arg: Box::new(int(1)),
        };
        let registry = Registry::with_builtins();

        // Without the capability: denied BEFORE the boundary is touched.
        let mut effects = MemEffects::default();
        let mut c = ctx(&registry, &mut effects, &[]);
        assert_eq!(
            eval_closed(&mut c, &term),
            Err(EvalError::CapabilityDenied {
                symbol: "io/echo".to_string(),
                capability: "io".to_string(),
            })
        );
        assert!(effects.begun.is_empty());

        // With it: intent begun and receipt committed around the effect.
        let mut effects = MemEffects::default();
        let mut c = ctx(&registry, &mut effects, &["io"]);
        assert_eq!(eval_closed(&mut c, &term).unwrap(), Value::Int(1));
        assert_eq!(effects.begun.len(), 1);
        assert_eq!(effects.committed, vec![("mem-0".to_string(), true)]);
    }

    #[test]
    fn deny_effects_blocks_external_calls_even_with_capability() {
        // Simulation posture: capability granted, but the boundary refuses —
        // a simulation branch must not reach production reality.
        let term = Term::Foreign {
            symbol: "io/echo".to_string(),
            arg: Box::new(int(1)),
        };
        let registry = Registry::with_builtins();
        let mut effects = DenyEffects;
        let mut c = ctx(&registry, &mut effects, &["io"]);
        assert!(matches!(
            eval_closed(&mut c, &term),
            Err(EvalError::BoundaryRefused(_))
        ));
    }
}
