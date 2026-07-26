//! `brain` — the command-line surface of the substrate.
//!
//! The CLI is a projection instrument: it renders and drives the graph, but
//! holds no state of its own. All state lives in the store (default `.brain/`).

mod tasks;

use brain_core::ids::NodeId;
use brain_core::object::{Object, Term};
use brain_index::{object_edges, replay, Index, MemIndex};
use brain_runtime::{
    eval_closed, value_to_json, CodeSource, EffectPort, EvalCtx, Registry, Value,
};
use brain_store::{now_ms, Store};
use serde_json::json;
use std::collections::BTreeSet;
use std::process::ExitCode;

const DEFAULT_FUEL: u64 = 1_000_000;

fn usage() -> &'static str {
    "brain — agent-native semantic substrate (scaffold)\n\
     \n\
     Usage:\n\
       brain init                         create a store in ./.brain\n\
       brain status                       objects, namespace, intent states\n\
       brain names                        list name -> node bindings\n\
       brain put-code <name> <term.json>  store a term and bind it\n\
       brain run <name> [--cap <c>]...    evaluate the code bound to a name\n\
       brain recover                      mark pending intents indeterminate\n\
       brain ingest <dir> [--prefix <p>]  twin an external source tree\n\
       brain refs <name|b3:hash>          who references this node (reverse edges)\n\
       brain deps <name|b3:hash>          what this node references (forward edges)\n\
       brain observations <name>          observations about a twinned entity\n\
       brain task check <task.json> <term.json>   check a solution, record evidence\n\
       brain demo                         run the end-to-end demonstration\n"
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("init") => cmd_init(),
        Some("status") => cmd_status(),
        Some("names") => cmd_names(),
        Some("put-code") => cmd_put_code(&args[1..]),
        Some("run") => cmd_run(&args[1..]),
        Some("recover") => cmd_recover(),
        Some("ingest") => cmd_ingest(&args[1..]),
        Some("refs") => cmd_refs(&args[1..]),
        Some("deps") => cmd_deps(&args[1..]),
        Some("observations") => cmd_observations(&args[1..]),
        Some("task") => cmd_task(&args[1..]),
        Some("demo") => cmd_demo(),
        _ => {
            eprint!("{}", usage());
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn open_store() -> Result<Store, String> {
    Store::open(".brain").map_err(|e| e.to_string())
}

fn cmd_init() -> Result<(), String> {
    let store = open_store()?;
    println!("store ready at {}", store.root().display());
    Ok(())
}

fn cmd_status() -> Result<(), String> {
    let store = open_store()?;
    let head = store.head().map_err(|e| e.to_string())?;
    println!("objects:    {}", store.count_objects().map_err(|e| e.to_string())?);
    println!("names:      {}", store.namespace().map_err(|e| e.to_string())?.len());
    println!(
        "head:       {}",
        head.map(|h| h.to_string()).unwrap_or_else(|| "(none)".to_string())
    );
    println!(
        "history:    {} namespace step(s)",
        store.namespace_history().map_err(|e| e.to_string())?.len()
    );
    println!(
        "intents:    {}",
        store.intents().summary().map_err(|e| e.to_string())?
    );
    Ok(())
}

fn cmd_names() -> Result<(), String> {
    let store = open_store()?;
    for (name, id) in store.namespace().map_err(|e| e.to_string())? {
        println!("{name}  ->  {id}");
    }
    Ok(())
}

fn cmd_put_code(args: &[String]) -> Result<(), String> {
    let (name, path) = match args {
        [name, path] => (name, path),
        _ => return Err("usage: brain put-code <name> <term.json>".to_string()),
    };
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let term: Term = serde_json::from_str(&text).map_err(|e| format!("invalid term: {e}"))?;
    let store = open_store()?;
    let id = store
        .put(&Object::Code { term })
        .map_err(|e| e.to_string())?;
    store.bind(name, id).map_err(|e| e.to_string())?;
    println!("{name}  ->  {id}");
    Ok(())
}

fn parse_caps(args: &[String]) -> BTreeSet<String> {
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

fn cmd_run(args: &[String]) -> Result<(), String> {
    let name = args.first().ok_or("usage: brain run <name> [--cap <c>]...")?;
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

fn run_node(store: &Store, id: &NodeId, caps: BTreeSet<String>) -> Result<Value, String> {
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

fn cmd_recover() -> Result<(), String> {
    let store = open_store()?;
    let marked = store.intents().recover().map_err(|e| e.to_string())?;
    if marked.is_empty() {
        println!("no pending intents; nothing to recover");
    } else {
        for intent in &marked {
            println!("indeterminate: {intent}");
        }
        println!(
            "{} intent(s) marked indeterminate — NOT retried; reconcile before re-attempting",
            marked.len()
        );
    }
    Ok(())
}

fn cmd_ingest(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("usage: brain ingest <dir> [--prefix <p>]")?;
    let mut prefix = "twin".to_string();
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        if a == "--prefix" {
            if let Some(p) = it.next() {
                prefix = p.clone();
            }
        }
    }
    let store = open_store()?;
    let report = brain_observe::ingest_dir(&store, std::path::Path::new(dir), &prefix)
        .map_err(|e| e.to_string())?;
    println!(
        "twinned {} file(s): {} entities, {} observations under '{prefix}/'",
        report.files, report.entities, report.observations
    );
    Ok(())
}

/// Resolve a CLI argument that may be a bound name or a literal b3: hash.
fn resolve_arg(store: &Store, arg: &str) -> Result<NodeId, String> {
    if arg.starts_with("b3:") {
        return NodeId::parse(arg).map_err(|e| e.to_string());
    }
    store
        .resolve(arg)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no binding for '{arg}'"))
}

/// Names currently bound to each node, for human-readable listings.
fn names_of(store: &Store) -> Result<std::collections::BTreeMap<NodeId, Vec<String>>, String> {
    let mut out: std::collections::BTreeMap<NodeId, Vec<String>> = Default::default();
    for (name, id) in store.namespace().map_err(|e| e.to_string())? {
        out.entry(id).or_default().push(name);
    }
    Ok(out)
}

fn kind_of(obj: &Object) -> &'static str {
    match obj {
        Object::Code { .. } => "code",
        Object::Spec { .. } => "spec",
        Object::Evidence { .. } => "evidence",
        Object::Capability { .. } => "capability",
        Object::Entity { .. } => "entity",
        Object::Observation { .. } => "observation",
        Object::Intent { .. } => "intent",
        Object::Receipt { .. } => "receipt",
        Object::Namespace { .. } => "namespace",
    }
}

fn describe(store: &Store, names: &std::collections::BTreeMap<NodeId, Vec<String>>, id: &NodeId) -> String {
    let kind = store.get(id).map(|o| kind_of(&o)).unwrap_or("missing");
    let bound = names
        .get(id)
        .map(|n| format!("  ({})", n.join(", ")))
        .unwrap_or_default();
    format!("{id:?}  {kind}{bound}")
}

fn build_index(store: &Store) -> Result<MemIndex, String> {
    let mut index = MemIndex::new();
    replay(store, &mut index).map_err(|e| e.to_string())?;
    Ok(index)
}

fn cmd_refs(args: &[String]) -> Result<(), String> {
    let arg = args.first().ok_or("usage: brain refs <name|b3:hash>")?;
    let store = open_store()?;
    let target = resolve_arg(&store, arg)?;
    let index = build_index(&store)?;
    let names = names_of(&store)?;
    let referrers = index.referrers(&target);
    if referrers.is_empty() {
        println!("nothing references {target:?}");
    } else {
        for id in referrers {
            println!("{}", describe(&store, &names, &id));
        }
    }
    Ok(())
}

fn cmd_deps(args: &[String]) -> Result<(), String> {
    let arg = args.first().ok_or("usage: brain deps <name|b3:hash>")?;
    let store = open_store()?;
    let target = resolve_arg(&store, arg)?;
    let obj = store.get(&target).map_err(|e| e.to_string())?;
    let names = names_of(&store)?;
    let edges = object_edges(&obj);
    if edges.is_empty() {
        println!("{target:?} references nothing");
    } else {
        for (kind, id) in edges {
            println!("{kind:?}  {}", describe(&store, &names, &id));
        }
    }
    Ok(())
}

fn cmd_observations(args: &[String]) -> Result<(), String> {
    let arg = args.first().ok_or("usage: brain observations <name>")?;
    let store = open_store()?;
    let node = resolve_arg(&store, arg)?;
    let stable = match store.get(&node).map_err(|e| e.to_string())? {
        Object::Entity { id, .. } => id,
        other => return Err(format!("'{arg}' is not an entity (found {})", kind_of(&other))),
    };
    let index = build_index(&store)?;
    let mut rows = Vec::new();
    for obs_id in index.observations_of(&stable) {
        if let Object::Observation { property, value, source, observed_at_ms, .. } =
            store.get(&obs_id).map_err(|e| e.to_string())?
        {
            rows.push((observed_at_ms, property, value, source));
        }
    }
    rows.sort();
    for (at, property, value, source) in rows {
        println!("{at}  {property} = {value}  [{source}]");
    }
    Ok(())
}

fn cmd_task(args: &[String]) -> Result<(), String> {
    let (task_path, term_path) = match args {
        [sub, task, term] if sub == "check" => (task, term),
        _ => return Err("usage: brain task check <task.json> <term.json>".to_string()),
    };
    let task: tasks::TaskDef = serde_json::from_str(
        &std::fs::read_to_string(task_path).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("invalid task: {e}"))?;
    let term: Term = serde_json::from_str(
        &std::fs::read_to_string(term_path).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("invalid term: {e}"))?;

    let store = open_store()?;
    let report = tasks::check(&store, &task, &term)?;

    println!("task: {}  —  {}", task.name, task.description);
    if !task.spec.is_null() {
        println!("spec: {}", task.spec);
    }
    for (i, r) in report.results.iter().enumerate() {
        let mark = if r.passed { "pass" } else { "FAIL" };
        println!("  case {i}: {mark}  {}  (fuel {})", r.detail, r.fuel_used);
    }
    println!("code:      {}", report.code_id);
    println!("evidence:  {}  (behavioral, passed={})", report.evidence_id, report.all_passed);
    println!("bound:     task/{}/latest", task.name);
    if !report.all_passed {
        return Err("task failed".to_string());
    }
    Ok(())
}

/// End-to-end demonstration of the living loop in miniature:
/// author -> store -> bind -> run (pure), then run (external effect) with a
/// capability, leaving intent + receipt in the graph.
fn cmd_demo() -> Result<(), String> {
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

struct StoreCode<'a> {
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
struct StoreEffects<'a> {
    store: &'a Store,
}

impl EffectPort for StoreEffects<'_> {
    fn begin(&mut self, symbol: &str, arg: &Value) -> Result<String, String> {
        let arg_hash = brain_core::canonical::hash_value(&value_to_json(arg))
            .map_err(|e| e.to_string())?;
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
