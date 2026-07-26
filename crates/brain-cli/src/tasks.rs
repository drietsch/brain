//! The Stage 1 authoring harness: tasks, checking, and evidence.
//!
//! A task is a unary-function contract: a description, an informal spec, and
//! input/expected cases. A candidate solution is a term (authored by an agent
//! against `docs/schema/term.schema.json`). Checking a solution:
//!
//! 1. stores the term in the graph (content identity = its hash),
//! 2. applies it to each case input in a *pure* context (no capabilities,
//!    effects denied — task checking is simulation posture),
//! 3. records the outcome as an `Evidence` object at the `Behavioral` level,
//!    attached to the code node. A test result is a fact about a hash.
//!
//! Case inputs/outputs are plain JSON: ints, strings, bools, null (unit),
//! objects (records), and `{"$variant": tag, "payload": ...}` for variants.
//! Arrays are unsupported until the calculus earns lists.

use brain_core::object::{Literal, Object, Term, VerificationLevel};
use brain_runtime::{eval_closed, value_to_json, DenyEffects, EvalCtx, Registry};
use brain_store::Store;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

const TASK_FUEL: u64 = 1_000_000;

#[derive(Deserialize)]
pub struct TaskDef {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub spec: serde_json::Value,
    pub cases: Vec<Case>,
}

#[derive(Deserialize)]
pub struct Case {
    pub arg: serde_json::Value,
    pub expect: serde_json::Value,
}

/// Convert a plain-JSON case value into a term of the calculus.
pub fn json_to_term(v: &serde_json::Value) -> Result<Term, String> {
    match v {
        serde_json::Value::Null => Ok(Term::Lit { value: Literal::Unit }),
        serde_json::Value::Bool(b) => Ok(Term::Lit { value: Literal::Bool { value: *b } }),
        serde_json::Value::Number(n) => n
            .as_i64()
            .map(|i| Term::Lit { value: Literal::Int { value: i } })
            .ok_or_else(|| "only integers are supported (no floats)".to_string()),
        serde_json::Value::String(s) => Ok(Term::Lit { value: Literal::Str { value: s.clone() } }),
        serde_json::Value::Object(map) => {
            if let (Some(serde_json::Value::String(tag)), Some(payload), 2) =
                (map.get("$variant"), map.get("payload"), map.len())
            {
                return Ok(Term::Variant {
                    tag: tag.clone(),
                    payload: Box::new(json_to_term(payload)?),
                });
            }
            let mut fields = BTreeMap::new();
            for (k, val) in map {
                fields.insert(k.clone(), json_to_term(val)?);
            }
            Ok(Term::Record { fields })
        }
        serde_json::Value::Array(_) => {
            Err("arrays are not supported in the scaffold calculus".to_string())
        }
    }
}

pub struct CaseResult {
    pub passed: bool,
    pub detail: String,
    pub fuel_used: u64,
}

pub struct CheckReport {
    pub code_id: brain_core::ids::NodeId,
    pub evidence_id: brain_core::ids::NodeId,
    pub results: Vec<CaseResult>,
    pub all_passed: bool,
}

/// Check a candidate term against a task and persist the outcome as evidence.
pub fn check(store: &Store, task: &TaskDef, solution: &Term) -> Result<CheckReport, String> {
    let code_id = store
        .put(&Object::Code { term: solution.clone() })
        .map_err(|e| e.to_string())?;

    let registry = Registry::with_builtins();
    let mut results = Vec::new();

    for case in &task.cases {
        let applied = Term::App {
            func: Box::new(solution.clone()),
            arg: Box::new(json_to_term(&case.arg)?),
        };
        // Simulation posture: pure fuel-bounded evaluation, effects denied.
        let mut effects = DenyEffects;
        let mut ctx = EvalCtx {
            fuel: TASK_FUEL,
            caps: BTreeSet::new(),
            registry: &registry,
            code: &brain_runtime::NoCode,
            effects: &mut effects,
        };
        let outcome = eval_closed(&mut ctx, &applied);
        let fuel_used = TASK_FUEL - ctx.fuel;
        results.push(match outcome {
            Ok(v) => {
                let got = value_to_json(&v);
                if got == case.expect {
                    CaseResult { passed: true, detail: format!("= {got}"), fuel_used }
                } else {
                    CaseResult {
                        passed: false,
                        detail: format!("expected {} but got {got}", case.expect),
                        fuel_used,
                    }
                }
            }
            Err(e) => CaseResult { passed: false, detail: e.to_string(), fuel_used },
        });
    }

    let all_passed = results.iter().all(|r| r.passed);
    let failures: Vec<String> = results
        .iter()
        .enumerate()
        .filter(|(_, r)| !r.passed)
        .map(|(i, r)| format!("case {i}: {}", r.detail))
        .collect();

    let evidence = Object::Evidence {
        subject: code_id,
        level: VerificationLevel::Behavioral,
        method: format!("task:{}", task.name),
        passed: all_passed,
        detail: if all_passed {
            format!("{} case(s) passed", results.len())
        } else {
            failures.join("; ")
        },
    };
    let evidence_id = store.put(&evidence).map_err(|e| e.to_string())?;
    store
        .bind(&format!("task/{}/latest", task.name), code_id)
        .map_err(|e| e.to_string())?;

    Ok(CheckReport { code_id, evidence_id, results, all_passed })
}
