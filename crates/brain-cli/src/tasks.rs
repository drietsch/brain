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
use brain_index::{replay, Index, MemIndex};
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
        serde_json::Value::Null => Ok(Term::Lit {
            value: Literal::Unit,
        }),
        serde_json::Value::Bool(b) => Ok(Term::Lit {
            value: Literal::Bool { value: *b },
        }),
        serde_json::Value::Number(n) => n
            .as_i64()
            .map(|i| Term::Lit {
                value: Literal::Int { value: i },
            })
            .ok_or_else(|| "only integers are supported (no floats)".to_string()),
        serde_json::Value::String(s) => Ok(Term::Lit {
            value: Literal::Str { value: s.clone() },
        }),
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
    /// True when prior passing evidence for this exact (code, task) pair was
    /// found in the graph and evaluation was skipped entirely.
    pub cached: bool,
}

/// Check a candidate term against a task and persist the outcome as evidence.
///
/// `task_key` must identify the task *content* (a hash of the task file), so
/// the evidence cache key is the (code hash, task content) pair — editing a
/// task's cases invalidates its cached verdicts automatically. Because the
/// store alpha-normalizes code, a re-authored solution that normalizes to an
/// already-attested hash skips evaluation: a test result is a fact about a
/// hash, forever. Only *passing* evidence short-circuits; failures re-run.
pub fn check(
    store: &Store,
    task: &TaskDef,
    task_key: &str,
    solution: &Term,
) -> Result<CheckReport, String> {
    let code_id = store
        .put(&Object::Code {
            term: solution.clone(),
        })
        .map_err(|e| e.to_string())?;
    let method = format!("task:{}@{}", task.name, task_key);

    let mut index = MemIndex::new();
    replay(store, &mut index).map_err(|e| e.to_string())?;
    for ev_id in index.evidence_for(&code_id) {
        if let Object::Evidence {
            method: m,
            passed: true,
            ..
        } = store.get(&ev_id).map_err(|e| e.to_string())?
        {
            if m == method {
                store
                    .bind(&format!("task/{}/latest", task.name), code_id)
                    .map_err(|e| e.to_string())?;
                return Ok(CheckReport {
                    code_id,
                    evidence_id: ev_id,
                    results: Vec::new(),
                    all_passed: true,
                    cached: true,
                });
            }
        }
    }

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
                    CaseResult {
                        passed: true,
                        detail: format!("= {got}"),
                        fuel_used,
                    }
                } else {
                    CaseResult {
                        passed: false,
                        detail: format!("expected {} but got {got}", case.expect),
                        fuel_used,
                    }
                }
            }
            Err(e) => CaseResult {
                passed: false,
                detail: e.to_string(),
                fuel_used,
            },
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
        method,
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

    Ok(CheckReport {
        code_id,
        evidence_id,
        results,
        all_passed,
        cached: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn increment_task() -> TaskDef {
        TaskDef {
            name: "inc".to_string(),
            description: "add one".to_string(),
            spec: json!(null),
            cases: vec![
                Case {
                    arg: json!(1),
                    expect: json!(2),
                },
                Case {
                    arg: json!(41),
                    expect: json!(42),
                },
            ],
        }
    }

    fn increment_solution(param: &str) -> Term {
        let mut fields = BTreeMap::new();
        fields.insert(
            "a".to_string(),
            Term::Var {
                name: param.to_string(),
            },
        );
        fields.insert(
            "b".to_string(),
            Term::Lit {
                value: Literal::Int { value: 1 },
            },
        );
        Term::Lam {
            param: param.to_string(),
            body: Box::new(Term::Foreign {
                symbol: "core/add".to_string(),
                arg: Box::new(Term::Record { fields }),
            }),
        }
    }

    #[test]
    fn passing_evidence_caches_across_alpha_equivalent_solutions() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let task = increment_task();

        let first = check(&store, &task, "tkey1", &increment_solution("n")).unwrap();
        assert!(!first.cached && first.all_passed);

        // Same task, alpha-renamed solution: normalizes to the same hash,
        // prior evidence attests it, evaluation is skipped.
        let second = check(&store, &task, "tkey1", &increment_solution("x")).unwrap();
        assert!(second.cached && second.all_passed);
        assert_eq!(first.code_id, second.code_id);
        assert_eq!(first.evidence_id, second.evidence_id);

        // Different task content (new key): the cache must NOT apply.
        let third = check(&store, &task, "tkey2", &increment_solution("n")).unwrap();
        assert!(!third.cached);
    }

    #[test]
    fn failing_evidence_never_caches() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let mut task = increment_task();
        task.cases[0].expect = json!(999); // unsatisfiable

        let first = check(&store, &task, "tkey", &increment_solution("n")).unwrap();
        assert!(!first.all_passed && !first.cached);
        let second = check(&store, &task, "tkey", &increment_solution("n")).unwrap();
        assert!(!second.cached, "failures must re-run, not cache");
    }
}
