//! `brain` — the command-line surface of the substrate.
//!
//! The CLI is a projection instrument: it renders and drives the graph, but
//! holds no state of its own. All state lives in the store (default `.brain/`).

mod notation;
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
       brain put-code <name> <term>       store a term (.json or .term) and bind it\n\
       brain notation <file>              convert term between .term notation and JSON\n\
       brain run <name> [--cap <c>]...    evaluate the code bound to a name\n\
       brain recover                      mark pending intents indeterminate\n\
       brain twin refresh <dir> [--prefix <p>]   observe a source tree, record drift\n\
       brain twin status <dir> [--prefix <p>]    report drift without writing\n\
       brain twin files <prefix>                 twinned files with freshness\n\
       brain twin symbols|imports|rdeps <name>   structure queries on a twinned file\n\
       brain twin search <substring>             find twinned entities by name\n\
       brain note <name> <text...>        attach a durable note to an entity\n\
       brain notes <name>                 read an entity's notes\n\
       brain ingest <dir> [--prefix <p>]  alias for twin refresh\n\
       brain pull <store-root>            replicate another store into this one\n\
       brain push <store-root>            replicate this store into another\n\
       brain refs <name|b3:hash>          who references this node (reverse edges)\n\
       brain evidence <name|b3:hash>      verification claims about a node\n\
       brain deps <name|b3:hash>          what this node references (forward edges)\n\
       brain observations <name>          observations about a twinned entity\n\
       brain task check <task.json> <term.json>   check a solution, record evidence\n\
       brain demo                         run the end-to-end demonstration\n"
}

fn main() -> ExitCode {
    // Die quietly on a closed pipe (`brain status | head`) instead of
    // panicking — restore the default Unix SIGPIPE behavior Rust masks.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("init") => cmd_init(),
        Some("status") => cmd_status(),
        Some("names") => cmd_names(),
        Some("put-code") => cmd_put_code(&args[1..]),
        Some("notation") => cmd_notation(&args[1..]),
        Some("run") => cmd_run(&args[1..]),
        Some("recover") => cmd_recover(),
        Some("ingest") => cmd_twin_refresh(&args[1..], true),
        Some("twin") => cmd_twin(&args[1..]),
        Some("note") => cmd_note(&args[1..]),
        Some("notes") => cmd_notes(&args[1..]),
        Some("pull") => cmd_sync(&args[1..], true),
        Some("push") => cmd_sync(&args[1..], false),
        Some("refs") => cmd_refs(&args[1..]),
        Some("evidence") => cmd_evidence(&args[1..]),
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
    let root = std::env::var("BRAIN_STORE").unwrap_or_else(|_| ".brain".to_string());
    Store::open(root).map_err(|e| e.to_string())
}

/// Open an existing store at an explicit path (for sync). Refuses to
/// conjure an empty store out of a typo.
fn open_existing_store(root: &str) -> Result<Store, String> {
    if !std::path::Path::new(root).join("objects").is_dir() {
        return Err(format!("no store at '{root}' (missing objects/)"));
    }
    Store::open(root).map_err(|e| e.to_string())
}

fn print_sync_report(report: &brain_store::sync::SyncReport) {
    println!(
        "objects: {} copied, {} already present",
        report.objects_copied, report.objects_present
    );
    println!(
        "names:   {} added, {} agreed",
        report.names_added, report.names_agreed
    );
    for (name, kept, incoming) in &report.conflicts {
        println!("CONFLICT {name}: kept {kept:?}, source's {incoming:?} bound as sync-conflict/{name}");
    }
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

/// Load a term from disk: `.term` files are compact notation, anything else
/// is the JSON encoding. Both parse into the same canonical Term.
fn load_term(path: &str) -> Result<Term, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    if path.ends_with(".term") {
        notation::parse_term(&text).map_err(|e| format!("invalid term notation: {e}"))
    } else {
        serde_json::from_str(&text).map_err(|e| format!("invalid term: {e}"))
    }
}

fn cmd_notation(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("usage: brain notation <file>")?;
    let term = load_term(path)?;
    if path.ends_with(".term") {
        println!(
            "{}",
            serde_json::to_string_pretty(&term).map_err(|e| e.to_string())?
        );
    } else {
        println!("{}", notation::print_term(&term));
    }
    Ok(())
}

fn cmd_put_code(args: &[String]) -> Result<(), String> {
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

fn parse_prefix(args: &[String]) -> String {
    let mut prefix = "twin".to_string();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--prefix" {
            if let Some(p) = it.next() {
                prefix = p.clone();
            }
        }
    }
    prefix
}

fn print_twin_report(report: &brain_observe::twin::TwinReport, wrote: bool) {
    for f in &report.added {
        println!("added    {f}");
    }
    for f in &report.changed {
        println!("changed  {f}");
    }
    for f in &report.deleted {
        println!("deleted  {f}");
    }
    let verb = if wrote { "recorded" } else { "would record" };
    println!(
        "{} unchanged; {verb} {} added, {} changed, {} deleted ({} symbols, {} relations)",
        report.unchanged,
        report.added.len(),
        report.changed.len(),
        report.deleted.len(),
        report.symbols,
        report.relations
    );
}

fn cmd_twin_refresh(args: &[String], write: bool) -> Result<(), String> {
    let dir = args
        .first()
        .ok_or("usage: brain twin refresh|status <dir> [--prefix <p>]")?;
    let prefix = parse_prefix(&args[1..]);
    let store = open_store()?;
    let path = std::path::Path::new(dir);
    let report = if write {
        brain_observe::twin::refresh(&store, path, &prefix)
    } else {
        brain_observe::twin::status(&store, path, &prefix)
    }
    .map_err(|e| e.to_string())?;
    print_twin_report(&report, write);
    Ok(())
}

/// Resolve a bound name to the entity's stable id.
fn entity_sid(store: &Store, name: &str) -> Result<brain_core::ids::StableId, String> {
    let node = resolve_arg(store, name)?;
    match store.get(&node).map_err(|e| e.to_string())? {
        Object::Entity { id, .. } => Ok(id),
        other => Err(format!("'{name}' is not an entity (found {})", kind_of(&other))),
    }
}

/// Distinct target entities of relations of `kind` leaving `sid`.
fn relation_targets(
    store: &Store,
    index: &MemIndex,
    sid: &brain_core::ids::StableId,
    kind: &str,
) -> Result<Vec<brain_core::ids::StableId>, String> {
    let mut out = Vec::new();
    for id in index.relations_from(sid, kind) {
        if let Object::Relation { to, .. } = store.get(&id).map_err(|e| e.to_string())? {
            if !out.contains(&to) {
                out.push(to);
            }
        }
    }
    Ok(out)
}

/// A human-readable label for an entity: its path, name, or raw stable id.
fn entity_label(store: &Store, index: &MemIndex, sid: &brain_core::ids::StableId) -> String {
    for node in index.entity_nodes(sid) {
        if let Ok(Object::Entity { labels, entity_kind, .. }) = store.get(&node) {
            if let Some(p) = labels.get("path").or_else(|| labels.get("name")) {
                return format!("{p} ({entity_kind})");
            }
        }
    }
    sid.to_string()
}

fn cmd_twin(args: &[String]) -> Result<(), String> {
    let usage = "usage: brain twin refresh|status|files|symbols|imports|rdeps|search ...";
    match args.first().map(String::as_str) {
        Some("refresh") => cmd_twin_refresh(&args[1..], true),
        Some("status") => cmd_twin_refresh(&args[1..], false),
        Some("files") => {
            let prefix = args.get(1).ok_or("usage: brain twin files <prefix>")?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let now = now_ms();
            for (name, node) in store.namespace().map_err(|e| e.to_string())? {
                let Some(rel) = name.strip_prefix(&format!("{prefix}/")) else { continue };
                let Ok(Object::Entity { id: sid, .. }) = store.get(&node) else { continue };
                let present = brain_observe::twin::latest(&index, &store, &sid, "present")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_else(|| "true".to_string());
                let age = brain_observe::twin::latest_at(&index, &store, &sid, "content_b3")
                    .map_err(|e| e.to_string())?
                    .map(|(at, _)| format!("{}s", now.saturating_sub(at) / 1000))
                    .unwrap_or_else(|| "?".to_string());
                let symbols = index.relations_from(&sid, "contains").len();
                let lang = brain_observe::twin::latest(&index, &store, &sid, "language")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default();
                let flag = if present == "false" { "  [deleted]" } else { "" };
                println!("{rel}  {lang}  {symbols} symbol(s)  observed {age} ago{flag}");
            }
            Ok(())
        }
        Some("symbols") => {
            let name = args.get(1).ok_or("usage: brain twin symbols <file-name>")?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = entity_sid(&store, name)?;
            for target in relation_targets(&store, &index, &sid, "contains")? {
                let line = brain_observe::twin::latest(&index, &store, &target, "line")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_else(|| "?".to_string());
                for node in index.entity_nodes(&target) {
                    if let Ok(Object::Entity { labels, .. }) = store.get(&node) {
                        let kind = labels.get("kind").cloned().unwrap_or_default();
                        let sym = labels.get("name").cloned().unwrap_or_default();
                        println!("{kind:<10} {sym}  (line {line})");
                        break;
                    }
                }
            }
            Ok(())
        }
        Some("imports") => {
            let name = args.get(1).ok_or("usage: brain twin imports <file-name>")?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = entity_sid(&store, name)?;
            for target in relation_targets(&store, &index, &sid, "imports")? {
                println!("{}", entity_label(&store, &index, &target));
            }
            Ok(())
        }
        Some("rdeps") => {
            let name = args.get(1).ok_or("usage: brain twin rdeps <file-name>")?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = entity_sid(&store, name)?;
            let mut froms = Vec::new();
            for id in index.relations_to(&sid, "imports") {
                if let Object::Relation { from, .. } = store.get(&id).map_err(|e| e.to_string())? {
                    if !froms.contains(&from) {
                        froms.push(from);
                    }
                }
            }
            if froms.is_empty() {
                println!("nothing imports {name}");
            }
            for from in froms {
                println!("{}", entity_label(&store, &index, &from));
            }
            Ok(())
        }
        Some("search") => {
            let needle = args.get(1).ok_or("usage: brain twin search <substring>")?;
            let store = open_store()?;
            for (name, id) in store.namespace().map_err(|e| e.to_string())? {
                if name.contains(needle.as_str()) {
                    println!("{name}  ->  {id:?}");
                }
            }
            Ok(())
        }
        _ => Err(usage.to_string()),
    }
}

fn cmd_note(args: &[String]) -> Result<(), String> {
    let (name, text) = match args {
        [name, rest @ ..] if !rest.is_empty() => (name, rest.join(" ")),
        _ => return Err("usage: brain note <name> <text...>".to_string()),
    };
    let store = open_store()?;
    let sid = entity_sid(&store, name)?;
    brain_observe::twin::add_note(&store, &sid, &text).map_err(|e| e.to_string())?;
    println!("noted on {name}");
    Ok(())
}

fn cmd_notes(args: &[String]) -> Result<(), String> {
    let name = args.first().ok_or("usage: brain notes <name>")?;
    let store = open_store()?;
    let index = build_index(&store)?;
    let sid = entity_sid(&store, name)?;
    let notes = brain_observe::twin::notes(&index, &store, &sid).map_err(|e| e.to_string())?;
    if notes.is_empty() {
        println!("no notes on {name}");
    }
    for (at, text) in notes {
        println!("{at}  {text}");
    }
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
        Object::Relation { .. } => "relation",
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

fn cmd_sync(args: &[String], pull: bool) -> Result<(), String> {
    let other_root = args
        .first()
        .ok_or("usage: brain pull|push <store-root>")?;
    let local = open_store()?;
    let other = open_existing_store(other_root)?;
    let report = if pull {
        brain_store::sync::pull(&local, &other)
    } else {
        brain_store::sync::pull(&other, &local)
    }
    .map_err(|e| e.to_string())?;
    print_sync_report(&report);
    Ok(())
}

fn cmd_evidence(args: &[String]) -> Result<(), String> {
    let arg = args.first().ok_or("usage: brain evidence <name|b3:hash>")?;
    let store = open_store()?;
    let target = resolve_arg(&store, arg)?;
    let index = build_index(&store)?;
    let evidence = index.evidence_for(&target);
    if evidence.is_empty() {
        println!("no evidence recorded for {target:?}");
        return Ok(());
    }
    for id in evidence {
        if let Object::Evidence { level, method, passed, detail, .. } =
            store.get(&id).map_err(|e| e.to_string())?
        {
            let mark = if passed { "pass" } else { "FAIL" };
            println!("{mark}  {level:?}  {method}  {detail}");
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
    let task_text = std::fs::read_to_string(task_path).map_err(|e| e.to_string())?;
    let task: tasks::TaskDef =
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
    let report = tasks::check(&store, &task, &task_key, &term)?;

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
