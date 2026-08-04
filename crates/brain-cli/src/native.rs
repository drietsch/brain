//! Native code: terms, closed evaluation, and the demo that exercises both.

use brain_core::ids::NodeId;
use brain_core::object::{Object, Term};
use brain_runtime::{eval_closed, value_to_json, CodeSource, EffectPort, EvalCtx, Registry, Value};
use brain_store::{now_ms, Store};
use serde_json::json;
use std::collections::BTreeSet;
use crate::support::*;

pub(crate) const DEFAULT_FUEL: u64 = 1_000_000;

/// Load a term from disk: `.term` files are compact notation, anything else
/// is the JSON encoding. Both parse into the same canonical Term.
pub(crate) fn load_term(path: &str) -> Result<Term, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    if path.ends_with(".term") {
        crate::notation::parse_term(&text).map_err(|e| format!("invalid term notation: {e}"))
    } else {
        serde_json::from_str(&text).map_err(|e| format!("invalid term: {e}"))
    }
}

pub(crate) fn cmd_notation(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("usage: brain notation <file>")?;
    let term = load_term(path)?;
    if path.ends_with(".term") {
        println!(
            "{}",
            serde_json::to_string_pretty(&term).map_err(|e| e.to_string())?
        );
    } else {
        println!("{}", crate::notation::print_term(&term));
    }
    Ok(())
}

pub(crate) fn cmd_put_code(args: &[String]) -> Result<(), String> {
    let (name, path) = match args {
        [name, path] => (name, path),
        _ => return Err("usage: brain put-code <name> <term>".to_string()),
    };
    let term = load_term(path)?;
    let store = open_store()?;
    let id = store
        .put(&Object::Code { term })
        .map_err(|e| e.to_string())?;
    store.bind(name, id).map_err(|e| e.to_string())?;
    println!("{name}  ->  {id}");
    Ok(())
}

pub(crate) fn parse_caps(args: &[String]) -> BTreeSet<String> {
    let mut caps = BTreeSet::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--cap" {
            if let Some(c) = it.next() {
                caps.insert(c.clone());
            }
        }
    }
    caps
}

pub(crate) fn cmd_run(args: &[String]) -> Result<(), String> {
    let name = args
        .first()
        .ok_or("usage: brain run <name> [--cap <c>]...")?;
    let caps = parse_caps(&args[1..]);
    let store = open_store()?;
    let id = store
        .resolve(name)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no binding for '{name}'"))?;
    let value = run_node(&store, &id, caps)?;
    println!("{}", value_to_json(&value));
    Ok(())
}

pub(crate) fn run_node(store: &Store, id: &NodeId, caps: BTreeSet<String>) -> Result<Value, String> {
    let term = match store.get(id).map_err(|e| e.to_string())? {
        Object::Code { term } => term,
        other => return Err(format!("{id} is not code (found {other:?})")),
    };
    let registry = Registry::with_builtins();
    let mut effects = StoreEffects { store };
    let mut ctx = EvalCtx {
        fuel: DEFAULT_FUEL,
        caps,
        registry: &registry,
        code: &StoreCode { store },
        effects: &mut effects,
    };
    eval_closed(&mut ctx, &term).map_err(|e| e.to_string())
}

pub(crate) fn cmd_task(args: &[String]) -> Result<(), String> {
    let (task_path, term_path) = match args {
        [sub, task, term] if sub == "check" => (task, term),
        _ => return Err("usage: brain task check <task.json> <term.json>".to_string()),
    };
    let task_text = std::fs::read_to_string(task_path).map_err(|e| e.to_string())?;
    let task: crate::tasks::TaskDef =
        serde_json::from_str(&task_text).map_err(|e| format!("invalid task: {e}"))?;
    // Cache key = task CONTENT, so editing a task invalidates its cached verdicts.
    let task_value: serde_json::Value =
        serde_json::from_str(&task_text).map_err(|e| e.to_string())?;
    let task_key = brain_core::canonical::hash_value(&task_value)
        .map_err(|e| e.to_string())?
        .to_hex()[..12]
        .to_string();
    let term = load_term(term_path)?;

    let store = open_store()?;
    let report = crate::tasks::check(&store, &task, &task_key, &term)?;

    println!("task: {}  —  {}", task.name, task.description);
    if !task.spec.is_null() {
        println!("spec: {}", task.spec);
    }
    if report.cached {
        println!("  cached: prior evidence attests this (code, task) pair — evaluation skipped");
    }
    for (i, r) in report.results.iter().enumerate() {
        let mark = if r.passed { "pass" } else { "FAIL" };
        println!("  case {i}: {mark}  {}  (fuel {})", r.detail, r.fuel_used);
    }
    println!("code:      {}", report.code_id);
    println!(
        "evidence:  {}  (behavioral, passed={})",
        report.evidence_id, report.all_passed
    );
    println!("bound:     task/{}/latest", task.name);
    if !report.all_passed {
        return Err("task failed".to_string());
    }
    Ok(())
}

/// End-to-end demonstration of the living loop in miniature:
/// author -> store -> bind -> run (pure), then run (external effect) with a
/// capability, leaving intent + receipt in the graph.
pub(crate) fn cmd_demo() -> Result<(), String> {
    let store = open_store()?;

    // (\x -> core/add {a: x, b: 2}) 40
    let add2: Term = serde_json::from_value(json!({
        "op": "app",
        "func": {
            "op": "lam", "param": "x",
            "body": {
                "op": "foreign", "symbol": "core/add",
                "arg": { "op": "record", "fields": {
                    "a": { "op": "var", "name": "x" },
                    "b": { "op": "lit", "value": { "type": "int", "value": 2 } }
                }}
            }
        },
        "arg": { "op": "lit", "value": { "type": "int", "value": 40 } }
    }))
    .map_err(|e| e.to_string())?;

    let echo: Term = serde_json::from_value(json!({
        "op": "foreign", "symbol": "io/echo",
        "arg": { "op": "lit", "value": { "type": "str", "value": "hello, graph" } }
    }))
    .map_err(|e| e.to_string())?;

    let add2_id = store
        .put(&Object::Code { term: add2 })
        .map_err(|e| e.to_string())?;
    let echo_id = store
        .put(&Object::Code { term: echo })
        .map_err(|e| e.to_string())?;
    store
        .bind_many(vec![
            ("demo/answer".to_string(), add2_id),
            ("demo/echo".to_string(), echo_id),
        ])
        .map_err(|e| e.to_string())?;
    println!("demo/answer  ->  {add2_id}");
    println!("demo/echo    ->  {echo_id}");

    let v = run_node(&store, &add2_id, BTreeSet::new())?;
    println!("run demo/answer (pure)          = {}", value_to_json(&v));

    // Without the capability the effect must be denied.
    match run_node(&store, &echo_id, BTreeSet::new()) {
        Err(e) if e.contains("capability denied") => {
            println!("run demo/echo without --cap io  = denied: {e}");
        }
        other => return Err(format!("expected capability denial, got {other:?}")),
    }

    let mut caps = BTreeSet::new();
    caps.insert("io".to_string());
    let v = run_node(&store, &echo_id, caps)?;
    println!("run demo/echo --cap io          = {}", value_to_json(&v));
    println!(
        "intents after demo              = {}",
        store.intents().summary().map_err(|e| e.to_string())?
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Store-backed boundary implementations
// ---------------------------------------------------------------------------

pub(crate) struct StoreCode<'a> {
    store: &'a Store,
}

impl CodeSource for StoreCode<'_> {
    fn code(&self, id: &NodeId) -> Result<Term, String> {
        match self.store.get(id).map_err(|e| e.to_string())? {
            Object::Code { term } => Ok(term),
            other => Err(format!("{id} is not code (found {other:?})")),
        }
    }
}

/// The real effect boundary: intents and receipts are graph objects, and the
/// durable intent log tracks their state across crashes.
pub(crate) struct StoreEffects<'a> {
    store: &'a Store,
}

impl EffectPort for StoreEffects<'_> {
    fn begin(&mut self, symbol: &str, arg: &Value) -> Result<String, String> {
        let arg_hash =
            brain_core::canonical::hash_value(&value_to_json(arg)).map_err(|e| e.to_string())?;
        let intent = Object::Intent {
            action: symbol.to_string(),
            arg_hash,
            capability: None,
            at_ms: now_ms(),
        };
        let id = self.store.put(&intent).map_err(|e| e.to_string())?;
        self.store.intents().begin(id).map_err(|e| e.to_string())?;
        Ok(id.to_string())
    }

    fn commit(&mut self, token: &str, result: &Result<Value, String>) {
        let Ok(intent_id) = NodeId::parse(token) else {
            return;
        };
        let (ok, detail) = match result {
            Ok(v) => (true, value_to_json(v).to_string()),
            Err(e) => (false, e.clone()),
        };
        let receipt = Object::Receipt {
            intent: intent_id,
            ok,
            detail,
            at_ms: now_ms(),
        };
        if let Ok(receipt_id) = self.store.put(&receipt) {
            let log = self.store.intents();
            let _ = if ok {
                log.confirm(intent_id, receipt_id)
            } else {
                log.fail(intent_id, receipt_id)
            };
        }
    }
}
