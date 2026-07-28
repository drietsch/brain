//! `brain` — the command-line surface of the substrate.
//!
//! The CLI is a projection instrument: it renders and drives the graph, but
//! holds no state of its own. All state lives in the store (default `.brain/`).

mod docsgen;
mod hooks;
mod manual;
mod notation;
mod tasks;

use brain_core::ids::NodeId;
use brain_core::object::{Object, Term};
use brain_index::{object_edges, Index, MemIndex};
use brain_runtime::{eval_closed, value_to_json, CodeSource, EffectPort, EvalCtx, Registry, Value};
use brain_store::{now_ms, Store};
use serde_json::json;
use std::collections::BTreeSet;
use std::process::ExitCode;

const DEFAULT_FUEL: u64 = 1_000_000;

fn usage() -> String {
    manual::usage_text()
}

/// `brain man` — the manual, projected from the same registry as usage().
fn cmd_man(args: &[String]) -> Result<(), String> {
    let page = manual::man_page();
    if let Some(i) = args.iter().position(|a| a == "--out") {
        let path = args.get(i + 1).ok_or("--out needs a path")?;
        std::fs::write(path, &page).map_err(|e| e.to_string())?;
        println!("wrote {path}");
        return Ok(());
    }
    if args.iter().any(|a| a == "--install") {
        let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
        let dir = std::path::Path::new(&home).join(".local/share/man/man1");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join("brain.1");
        std::fs::write(&path, &page).map_err(|e| e.to_string())?;
        println!("installed {} — try: man brain", path.display());
        return Ok(());
    }
    print!("{page}");
    Ok(())
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
        Some("adr") => cmd_doc(&args[1..], brain_observe::docs::DocKind::Decision),
        Some("plan") => cmd_doc(&args[1..], brain_observe::docs::DocKind::Plan),
        Some("skill") => cmd_agent_doc(&args[1..], brain_observe::agents::AgentDocKind::Skill),
        Some("agentcfg") => cmd_agent_doc(&args[1..], brain_observe::agents::AgentDocKind::Config),
        Some("template") => cmd_template(&args[1..]),
        Some("artifact") => cmd_artifact(&args[1..]),
        Some("asset") => cmd_asset(&args[1..]),
        Some("instructions") => cmd_instructions(&args[1..]),
        Some("tidy") => cmd_tidy(&args[1..]),
        Some("deliverable") => cmd_deliverable(&args[1..]),
        Some("feature") => cmd_feature(&args[1..]),
        Some("done") => cmd_done(&args[1..]),
        Some("testrun") => cmd_testrun(&args[1..]),
        Some("sessions") => cmd_sessions(&args[1..]),
        Some("change") => cmd_change(&args[1..]),
        Some("bench") => cmd_bench(&args[1..]),
        Some("relation") => cmd_relation(&args[1..]),
        Some("wake") => cmd_wake(&args[1..]),
        Some("attend") => cmd_attend(&args[1..]),
        Some("spine") => cmd_spine(&args[1..]),
        Some("sleep") => cmd_sleep(&args[1..]),
        Some("related") => cmd_related(&args[1..]),
        Some("eyes") => cmd_eyes(&args[1..]),
        Some("docs") => docsgen::cmd_docs(&args[1..], open_store),
        Some("hook") => hooks::cmd_hook(&args[1..], open_store),
        Some("watch") => cmd_watch(&args[1..]),
        Some("man") => cmd_man(&args[1..]),
        Some("version") | Some("--version") | Some("-V") => {
            println!("brain {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
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
        // Deliberate gate refusals exit 3 so hooks can distinguish "brain
        // refused" (block the commit) from "brain broke" (fail open).
        Err(e) if e.starts_with("refused:") => {
            eprintln!("{e}");
            ExitCode::from(3)
        }
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
        println!(
            "CONFLICT {name}: kept {kept:?}, source's {incoming:?} bound as sync-conflict/{name}"
        );
    }
}

fn cmd_init() -> Result<(), String> {
    let store = open_store()?;
    let seeded = brain_observe::templates::seed(&store).map_err(|e| e.to_string())?;
    println!("store ready at {}", store.root().display());
    if seeded > 0 {
        println!(
            "seeded {} deliverable templates under brain/templates/",
            brain_observe::templates::DEFAULTS.len()
        );
    }
    Ok(())
}

fn cmd_status() -> Result<(), String> {
    let store = open_store()?;
    let head = store.head().map_err(|e| e.to_string())?;
    println!(
        "objects:    {}",
        store.count_objects().map_err(|e| e.to_string())?
    );
    println!(
        "names:      {}",
        store.namespace().map_err(|e| e.to_string())?.len()
    );
    println!(
        "head:       {}",
        head.map(|h| h.to_string())
            .unwrap_or_else(|| "(none)".to_string())
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
    // Governed changes whose intents just went indeterminate get marked too.
    for (name, node) in store.namespace().map_err(|e| e.to_string())? {
        if let Ok(Object::Entity { entity_kind, .. }) = store.get(&node) {
            if entity_kind == "repo" {
                for slug in
                    brain_observe::govern::reconcile(&store, &name).map_err(|e| e.to_string())?
                {
                    println!("change '{slug}' ({name}) marked indeterminate");
                }
            }
        }
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
    for f in &report.docs {
        println!("doc      {f}");
    }
    let verb = if wrote { "recorded" } else { "would record" };
    let retracted = if report.retracted > 0 {
        format!(", {} edge(s) retracted", report.retracted)
    } else {
        String::new()
    };
    println!(
        "{} unchanged; {verb} {} added, {} changed, {} deleted ({} symbols, {} relations, {} docs{retracted})",
        report.unchanged,
        report.added.len(),
        report.changed.len(),
        report.deleted.len(),
        report.symbols,
        report.relations,
        report.docs.len()
    );
}

fn cmd_twin_refresh(args: &[String], write: bool) -> Result<(), String> {
    let dir = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("usage: brain twin refresh|status <dir> [--prefix <p>] [--full]")?;
    let prefix = parse_prefix(&args[1..]);
    let full = args.iter().any(|a| a == "--full");
    let store = open_store()?;
    let path = std::path::Path::new(dir);
    let report = if write && full {
        brain_observe::twin::refresh_full(&store, path, &prefix)
    } else if write {
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
        other => Err(format!(
            "'{name}' is not an entity (found {})",
            kind_of(&other)
        )),
    }
}

/// Distinct target entities of live relations of `kind` leaving `sid`.
fn relation_targets(
    store: &Store,
    index: &MemIndex,
    sid: &brain_core::ids::StableId,
    kind: &str,
) -> Result<Vec<brain_core::ids::StableId>, String> {
    let mut out = Vec::new();
    for (_, to) in
        brain_observe::twin::live_from(index, store, sid, kind).map_err(|e| e.to_string())?
    {
        if !out.contains(&to) {
            out.push(to);
        }
    }
    Ok(out)
}

/// Resolve a name to an entity sid: a bound name first (files, repo),
/// then the slug of any doc-ish kind under the prefix.
fn resolve_entity(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    name: &str,
) -> Result<brain_core::ids::StableId, String> {
    if let Ok(sid) = entity_sid(store, name) {
        return Ok(sid);
    }
    brain_observe::features::resolve_target(store, index, prefix, name)
        .map_err(|e| e.to_string())?
        .map(|(sid, _)| sid)
        .ok_or_else(|| format!("no entity named '{name}' (tried bound names and {prefix} slugs)"))
}

/// Positional arguments: flags and the values they consume are dropped.
///
/// Filtering only on a leading `--` leaves a flag's value behind, so
/// `relation retract a b c --prefix p` looked like four positionals and
/// was rejected.
fn positional(args: &[String]) -> Vec<&String> {
    let mut out = Vec::new();
    let mut skip = false;
    for arg in args {
        if skip {
            skip = false;
            continue;
        }
        if let Some(flag) = arg.strip_prefix("--") {
            // Only value-taking flags swallow the next argument.
            skip = matches!(flag, "prefix" | "why" | "note" | "title" | "kind" | "file");
            continue;
        }
        out.push(arg);
    }
    out
}

fn cmd_relation(args: &[String]) -> Result<(), String> {
    let usage = "usage: brain relation retract <from> <predicate> <to> [--prefix <p>]\n       brain relation list <name> [--all] [--prefix <p>]";
    match args.first().map(String::as_str) {
        Some("retract") => {
            let pos = positional(&args[1..]);
            let [from, predicate, to] = pos.as_slice() else {
                return Err(usage.into());
            };
            let prefix = parse_prefix(&args[1..]);
            let store = open_store()?;
            let index = build_index(&store)?;
            let from_sid = resolve_entity(&store, &index, &prefix, from)?;
            let to_sid = resolve_entity(&store, &index, &prefix, to)?;
            if brain_observe::twin::retract(&store, &from_sid, predicate, &to_sid)
                .map_err(|e| e.to_string())?
            {
                println!("retracted {from} -{predicate}-> {to}");
            } else {
                println!("nothing to retract: no live {predicate} edge {from} -> {to}");
            }
            Ok(())
        }
        Some("list") => {
            let name = args.get(1).filter(|a| !a.starts_with("--")).ok_or(usage)?;
            let all = args.iter().any(|a| a == "--all");
            let prefix = parse_prefix(&args[1..]);
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = resolve_entity(&store, &index, &prefix, name)?;
            let mut seen: std::collections::BTreeSet<(String, String, String)> =
                std::collections::BTreeSet::new();
            let mut shown = 0usize;
            for id in store.put_history().map_err(|e| e.to_string())? {
                let Ok(Object::Relation {
                    from,
                    predicate,
                    to,
                    ..
                }) = store.get(&id)
                else {
                    continue;
                };
                let (out, other) = if from == sid {
                    (true, to.clone())
                } else if to == sid {
                    (false, from.clone())
                } else {
                    continue;
                };
                let live = brain_index::edge_active(&*index, &store, &from, &predicate, &to)
                    .map_err(|e| e.to_string())?;
                if !live && !all {
                    continue;
                }
                let label = entity_label(&store, &index, &other);
                let dir = if out { "->" } else { "<-" };
                let flag = if live { "" } else { "  [retracted]" };
                if seen.insert((dir.to_string(), predicate.clone(), label.clone())) {
                    println!("{dir} {predicate:<14} {label}{flag}");
                    shown += 1;
                }
            }
            if shown == 0 {
                println!("no {}relations for {name}", if all { "" } else { "live " });
            }
            Ok(())
        }
        _ => Err(usage.into()),
    }
}

/// A human-readable label for an entity: its path, name, or raw stable id.
fn entity_label(store: &Store, index: &MemIndex, sid: &brain_core::ids::StableId) -> String {
    for node in index.entity_nodes(sid) {
        if let Ok(Object::Entity {
            labels,
            entity_kind,
            ..
        }) = store.get(&node)
        {
            if let Some(p) = labels.get("path").or_else(|| labels.get("name")) {
                return format!("{p} ({entity_kind})");
            }
        }
    }
    sid.to_string()
}

fn cmd_twin(args: &[String]) -> Result<(), String> {
    let usage = "usage: brain twin refresh|status|files|symbols|imports|rdeps|search|insights ...";
    match args.first().map(String::as_str) {
        Some("refresh") => cmd_twin_refresh(&args[1..], true),
        Some("status") => cmd_twin_refresh(&args[1..], false),
        Some("files") => {
            let prefix = args.get(1).ok_or("usage: brain twin files <prefix>")?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let now = now_ms();
            for (name, node) in store.namespace().map_err(|e| e.to_string())? {
                let Some(rel) = name.strip_prefix(&format!("{prefix}/")) else {
                    continue;
                };
                let Ok(Object::Entity { id: sid, .. }) = store.get(&node) else {
                    continue;
                };
                let present = brain_observe::twin::latest(&index, &store, &sid, "present")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_else(|| "true".to_string());
                let age = brain_observe::twin::latest_at(&index, &store, &sid, "content_b3")
                    .map_err(|e| e.to_string())?
                    .map(|(at, _)| format!("{}s", now.saturating_sub(at) / 1000))
                    .unwrap_or_else(|| "?".to_string());
                let symbols = brain_observe::twin::live_from(&index, &store, &sid, "contains")
                    .map_err(|e| e.to_string())?
                    .len();
                let lang = brain_observe::twin::latest(&index, &store, &sid, "language")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default();
                let flag = if present == "false" {
                    let moved = brain_observe::twin::live_from(&index, &store, &sid, "renamed_to")
                        .map_err(|e| e.to_string())?;
                    match moved.first() {
                        Some((_, to)) => {
                            format!("  [moved to {}]", entity_label(&store, &index, to))
                        }
                        None => "  [deleted]".to_string(),
                    }
                } else {
                    String::new()
                };
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
        Some(op @ ("imports" | "rdeps")) => {
            let name = args
                .get(1)
                .filter(|a| !a.starts_with("--"))
                .ok_or("usage: brain twin imports|rdeps <file-name> [--transitive]")?;
            let reverse = op == "rdeps";
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = entity_sid(&store, name)?;
            if args.iter().any(|a| a == "--transitive") {
                // cortex's recursive walk: the full (blast) radius.
                let reached = index
                    .reach(&store, &sid, "imports", reverse, 64)
                    .map_err(|e| e.to_string())?;
                if reached.is_empty() {
                    println!("nothing, transitively");
                }
                for (target, depth) in reached {
                    println!(
                        "{}{}",
                        "  ".repeat(depth - 1),
                        entity_label(&store, &index, &target)
                    );
                }
                return Ok(());
            }
            let mut targets = Vec::new();
            let live = if reverse {
                brain_observe::twin::live_to(&index, &store, &sid, "imports")
            } else {
                brain_observe::twin::live_from(&index, &store, &sid, "imports")
            }
            .map_err(|e| e.to_string())?;
            for (_, t) in live {
                if !targets.contains(&t) {
                    targets.push(t);
                }
            }
            if targets.is_empty() {
                println!(
                    "nothing {} {name}",
                    if reverse { "imports" } else { "imported by" }
                );
            }
            for t in targets {
                println!("{}", entity_label(&store, &index, &t));
            }
            Ok(())
        }
        Some("backfill") => {
            let dir = args
                .get(1)
                .filter(|a| !a.starts_with("--"))
                .ok_or("usage: brain twin backfill <dir> [--prefix <p>] [--max-commits N]")?;
            let prefix = parse_prefix(&args[2..]);
            let max = args
                .iter()
                .position(|a| a == "--max-commits")
                .and_then(|i| args.get(i + 1))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let store = open_store()?;
            let report =
                brain_observe::backfill::backfill(&store, std::path::Path::new(dir), &prefix, max)
                    .map_err(|e| e.to_string())?;
            println!(
                "backfilled {} commit(s): {} file version(s), {} deletion(s), {} blob(s) skipped, {} object(s) written",
                report.commits,
                report.file_versions,
                report.deletions,
                report.skipped_blobs,
                report.objects_written
            );
            println!("history is in the graph: churn, `twin at <old-commit>`, and co-change now reach the past");
            Ok(())
        }
        Some("at") => {
            let (prefix, when) = match (args.get(1), args.get(2)) {
                (Some(p), Some(w)) => (p, w),
                _ => return Err("usage: brain twin at <prefix> <ms|30m|2h|1d|git-commit>".into()),
            };
            let store = open_store()?;
            let index = build_index(&store)?;
            let t = resolve_when(&store, &index, prefix, when)?;
            let files = brain_observe::twin::files_at(&store, &index, prefix, t)
                .map_err(|e| e.to_string())?;
            println!("== {prefix} as of {t} ({} file(s)) ==", files.len());
            for (rel, hash) in files {
                println!("{}  {rel}", &hash[..12]);
            }
            Ok(())
        }
        Some("insights") => {
            let prefix = args.get(1).ok_or("usage: brain twin insights <prefix>")?;
            let store = open_store()?;
            let ins = brain_observe::twin::insights(&store, prefix).map_err(|e| e.to_string())?;
            let now = now_ms();
            println!("== twin insights: {prefix} ==");
            {
                // The last consolidated memory, if the twin has ever slept.
                let index = build_index(&store)?;
                let repo = brain_core::ids::StableId::derive(&["repo", prefix.as_str()]);
                if let Some((at, s)) =
                    brain_observe::twin::latest_at(&index, &store, &repo, "session_summary")
                        .map_err(|e| e.to_string())?
                {
                    let age = now.saturating_sub(at) / 1000;
                    println!("last sleep ({age}s ago): {s}");
                }
            }
            println!(
                "files: {} present, {} deleted   symbols: {}   relations: {}",
                ins.files, ins.deleted_files, ins.symbols, ins.relations
            );
            if let (Some(branch), Some(commit)) = (&ins.git_branch, &ins.git_commit) {
                println!("git: {branch} @ {}", &commit[..commit.len().min(12)]);
            }
            if ins.test_files + ins.tests_declared > 0 {
                let last = ins
                    .last_run
                    .map(|(at, total, passed, failed)| {
                        let age = now.saturating_sub(at) / 1000;
                        let verdict = if failed == 0 { "ok" } else { "FAILED" };
                        format!("; last run {age}s ago: {verdict} ({passed}/{total} passed, {failed} failed)")
                    })
                    .unwrap_or_else(|| "; no runs imported".to_string());
                println!(
                    "tests: {} test file(s), {} declared{last}",
                    ins.test_files, ins.tests_declared
                );
            }
            // Rendering truncates; the data never does. Every shortened
            // list says so — a truncated count must never pose as a total.
            const SHOW: usize = 5;
            let showing = |total: usize, what: &str| {
                if total > SHOW {
                    println!("  … showing {SHOW} of {total} {what}");
                }
            };
            if !ins.failing.is_empty() {
                println!("failing tests ({}):", ins.failing.len());
                for name in ins.failing.iter().take(SHOW) {
                    println!("  ✗ {name}");
                }
                showing(ins.failing.len(), "failing");
            }
            let list = |title: &str, items: &[(String, usize)], unit: &str| {
                if !items.is_empty() {
                    println!("{title}:");
                    for (name, n) in items.iter().take(SHOW) {
                        println!("  {n:>4} {unit}  {name}");
                    }
                    if items.len() > SHOW {
                        println!("  … showing {SHOW} of {}", items.len());
                    }
                }
            };
            if !ins.churn.is_empty() {
                println!("churn (most edited):");
                for (name, n) in ins.churn.iter().take(SHOW) {
                    let tag = if ins.decided.contains(name) {
                        "  [decided]"
                    } else {
                        ""
                    };
                    println!("  {n:>4} versions  {name}{tag}");
                }
                showing(ins.churn.len(), "churned files");
            }
            list("hubs (most imported)", &ins.hubs, "importers");
            list(
                "untested hubs (imported, no tests)",
                &ins.untested_hubs,
                "importers",
            );
            list("largest (symbols declared)", &ins.largest, "symbols");
            list(
                "external deps (unresolved imports)",
                &ins.external_modules,
                "uses",
            );
            if !ins.decisions.is_empty() {
                println!("decisions (ADRs, {} active):", ins.decisions.len());
                for (slug, title, status) in ins.decisions.iter().take(SHOW) {
                    println!("  [{status}] {slug}: {title}");
                }
                showing(ins.decisions.len(), "decisions");
            }
            if !ins.plans.is_empty() {
                println!("plans ({} active):", ins.plans.len());
                for (slug, title) in ins.plans.iter().take(SHOW) {
                    println!("  {slug}: {title}");
                }
                showing(ins.plans.len(), "plans");
            }
            if !ins.skills.is_empty() {
                println!("agent skills:");
                for (slug, agent, desc) in ins.skills.iter().take(SHOW) {
                    println!("  [{agent}] {slug}: {desc}");
                }
                showing(ins.skills.len(), "skills");
            }
            if !ins.agent_configs.is_empty() {
                println!("agent config:");
                for (slug, agent, role) in ins.agent_configs.iter().take(SHOW) {
                    println!("  [{agent}] {slug} ({role})");
                }
                showing(ins.agent_configs.len(), "configs");
            }
            if !ins.custom_artifacts.is_empty() {
                let parts: Vec<String> = ins
                    .custom_artifacts
                    .iter()
                    .map(|(k, n)| format!("{k} ×{n}"))
                    .collect();
                println!(
                    "custom artifacts (graph-defined kinds): {}",
                    parts.join(", ")
                );
            }
            if !ins.features.is_empty() {
                println!("features (progress):");
                for feature in &ins.features {
                    let counted = if feature.by_parts { "parts" } else { "linked" };
                    println!(
                        "  [{}] {}  {} {counted}",
                        feature.status, feature.slug, feature.fraction
                    );
                }
            }
            if !ins.nonconforming.is_empty() {
                println!("nonconforming docs (template contract):");
                for (slug, kind, missing) in &ins.nonconforming {
                    println!("  {slug} ({kind}): missing {missing}");
                }
            }
            if !ins.stale_docs.is_empty() {
                println!("possibly stale docs (mentioned files changed since):");
                for d in ins.stale_docs.iter().take(SHOW) {
                    println!(
                        "  [{}] {} ({}): {}",
                        d.severity.as_str(),
                        d.slug,
                        d.kind,
                        d.changed.join(", ")
                    );
                }
                showing(ins.stale_docs.len(), "stale docs");
            }
            if !ins.notes.is_empty() {
                println!("recent notes:");
                for (at, entity, text) in ins.notes.iter().take(SHOW) {
                    let age = now.saturating_sub(*at) / 1000;
                    println!("  [{age}s ago] {entity}: {text}");
                }
                showing(ins.notes.len(), "notes");
            }
            {
                let index = build_index(&store)?;
                let findings = brain_observe::coherence::check(&store, &index, prefix)
                    .map_err(|e| e.to_string())?;
                if !findings.is_empty() {
                    println!("coherence findings:");
                    for f in findings.iter().take(SHOW) {
                        println!("  {f}");
                    }
                    showing(findings.len(), "findings");
                }
            }
            if ins.series.len() > 1 {
                println!("growth (files/symbols/relations over refreshes):");
                for (at, f, s, r) in &ins.series {
                    let age = now.saturating_sub(*at) / 1000;
                    println!("  -{age:>6}s  {f} files  {s} symbols  {r} relations");
                }
            }
            Ok(())
        }
        Some("tests") => {
            let prefix = args.get(1).ok_or("usage: brain twin tests <prefix>")?;
            let store = open_store()?;
            let index = build_index(&store)?;
            for (name, node) in store.namespace().map_err(|e| e.to_string())? {
                let Some(rel) = name.strip_prefix(&format!("{prefix}/")) else {
                    continue;
                };
                let Ok(Object::Entity { id: sid, .. }) = store.get(&node) else {
                    continue;
                };
                let Some(framework) =
                    brain_observe::twin::latest(&index, &store, &sid, "test_framework")
                        .map_err(|e| e.to_string())?
                else {
                    continue;
                };
                let declared = brain_observe::twin::latest(&index, &store, &sid, "tests_declared")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_else(|| "0".to_string());
                let role = brain_observe::twin::latest(&index, &store, &sid, "file_role")
                    .map_err(|e| e.to_string())?
                    .map(|_| "test file")
                    .unwrap_or("inline tests");
                let covers = relation_targets(&store, &index, &sid, "covers")?;
                let covering = if covers.is_empty() {
                    String::new()
                } else {
                    format!(
                        "  covers {}",
                        covers
                            .iter()
                            .map(|t| entity_label(&store, &index, t))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                println!("{rel}  [{framework}] {declared} test(s), {role}{covering}");
            }
            let failing = brain_observe::testing::failing_cases(&store, &index, prefix)
                .map_err(|e| e.to_string())?;
            if !failing.is_empty() {
                println!("failing now:");
                for name in failing {
                    println!("  ✗ {name}");
                }
            }
            Ok(())
        }
        Some("stale") => {
            let prefix = args.get(1).ok_or("usage: brain twin stale <prefix>")?;
            let store = open_store()?;
            let ins = brain_observe::twin::insights(&store, prefix).map_err(|e| e.to_string())?;
            if ins.stale_docs.is_empty() {
                println!("no stale docs under {prefix}");
            }
            for d in &ins.stale_docs {
                println!(
                    "[{}] {} ({}) — changed since doc updated or acknowledged:",
                    d.severity.as_str(),
                    d.slug,
                    d.kind
                );
                for f in &d.changed {
                    println!("  {f}");
                }
            }
            let warns = ins
                .stale_docs
                .iter()
                .filter(|d| d.severity == brain_observe::twin::Severity::Warn)
                .count();
            if !ins.stale_docs.is_empty() {
                println!(
                    "({warns} warn, {} info; reviewed-and-still-accurate? `brain adr|plan|artifact ack`)",
                    ins.stale_docs.len() - warns
                );
            }
            Ok(())
        }
        Some("config") => {
            let usage = "usage: brain twin config <prefix> [--add-extensions csv]";
            let prefix = args.get(1).filter(|a| !a.starts_with("--")).ok_or(usage)?;
            let add: Vec<String> = args
                .iter()
                .position(|a| a == "--add-extensions")
                .and_then(|i| args.get(i + 1))
                .map(|v| vec![v.clone()])
                .unwrap_or_default();
            let store = open_store()?;
            if add.is_empty() {
                let index = build_index(&store)?;
                let repo = brain_core::ids::StableId::derive(&["repo", prefix.as_str()]);
                let exts = brain_observe::twin::latest(&index, &store, &repo, "ingest_extensions")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_else(|| "(none beyond built-ins)".to_string());
                println!("extra ingest extensions for {prefix}: {exts}");
                return Ok(());
            }
            match brain_observe::twin::add_ingest_extensions(&store, prefix, &add)
                .map_err(|e| e.to_string())?
            {
                Some(csv) => println!(
                    "extra ingest extensions for {prefix}: {csv} (next refresh ingests them)"
                ),
                None => println!("no change — already taught"),
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

/// `brain adr ...` / `brain plan ...` — decisions and plans in the twin.
fn cmd_doc(args: &[String], kind: brain_observe::docs::DocKind) -> Result<(), String> {
    use brain_observe::{docs, twin};
    let noun = kind.as_str();
    let cmd = match kind {
        docs::DocKind::Decision => "adr",
        docs::DocKind::Plan => "plan",
    };
    let usage = format!(
        "usage: brain {cmd} add <md-file> --prefix <p> [--title T]{} | \
         brain {cmd} list <prefix> [--all] | brain {cmd} show <prefix> <slug>{}",
        if cmd == "adr" { " [--status S]" } else { "" },
        if cmd == "plan" {
            " | brain plan done|abandon|reopen <prefix> <slug> [--why R]"
        } else {
            ""
        }
    );
    match args.first().map(String::as_str) {
        Some(op @ ("done" | "abandon" | "reopen")) if cmd == "plan" => {
            let (prefix, slug) = match (args.get(1), args.get(2)) {
                (Some(p), Some(s)) if !p.starts_with("--") && !s.starts_with("--") => (p, s),
                _ => return Err(usage),
            };
            let why = args
                .iter()
                .position(|a| a == "--why")
                .and_then(|i| args.get(i + 1))
                .map(String::as_str);
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = brain_core::ids::StableId::derive(&[noun, prefix, slug]);
            if index.entity_nodes(&sid).is_empty() {
                return Err(format!("no {noun} '{slug}' under {prefix}"));
            }
            let state = match op {
                "done" => brain_observe::lifecycle::Lifecycle::Done,
                "abandon" => brain_observe::lifecycle::Lifecycle::Abandoned,
                _ => brain_observe::lifecycle::Lifecycle::Active,
            };
            let wrote = brain_observe::lifecycle::set(&store, &index, &sid, state, why)
                .map_err(|e| e.to_string())?;
            let verb = if wrote { "now" } else { "already" };
            println!("plan '{slug}' {verb} {}", state.as_str());
            Ok(())
        }
        Some("ack") => {
            let (prefix, slug) = match (args.get(1), args.get(2)) {
                (Some(p), Some(s)) if !p.starts_with("--") && !s.starts_with("--") => (p, s),
                _ => return Err(usage),
            };
            let note = args
                .iter()
                .position(|a| a == "--note")
                .and_then(|i| args.get(i + 1))
                .map(String::as_str)
                .unwrap_or("reviewed, still accurate");
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = brain_core::ids::StableId::derive(&[noun, prefix, slug]);
            if index.entity_nodes(&sid).is_empty() {
                return Err(format!("no {noun} '{slug}' under {prefix}"));
            }
            brain_observe::twin::ack(&store, &sid, note).map_err(|e| e.to_string())?;
            println!("{noun} '{slug}' acknowledged — staleness clock reset");
            Ok(())
        }
        Some("add") => {
            let mut file = None;
            let mut prefix = None;
            let mut title = None;
            let mut status = None;
            let mut it = args[1..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--prefix" => prefix = it.next().cloned(),
                    "--title" => title = it.next().cloned(),
                    "--status" if cmd == "adr" => status = it.next().cloned(),
                    other if file.is_none() && !other.starts_with("--") => {
                        file = Some(other.to_string())
                    }
                    other => return Err(format!("unexpected argument '{other}'\n{usage}")),
                }
            }
            let file = file.ok_or_else(|| usage.clone())?;
            let prefix = prefix.ok_or_else(|| format!("--prefix is required\n{usage}"))?;
            let content =
                std::fs::read_to_string(&file).map_err(|e| format!("cannot read '{file}': {e}"))?;
            let slug = docs::slug_of(&file);
            let meta =
                docs::parse_content(kind, &slug, &content, title.as_deref(), status.as_deref());
            let store = open_store()?;
            let out = twin::add_doc(&store, &prefix, &meta, &content, "claude-code")
                .map_err(|e| e.to_string())?;
            let state = if out.wrote {
                "recorded"
            } else {
                "already recorded (unchanged)"
            };
            let status_part = meta
                .status
                .as_deref()
                .map(|s| format!(", status: {s}"))
                .unwrap_or_default();
            println!(
                "{noun} '{slug}' {state} under {prefix} (title: {}{status_part}, {} mention(s))",
                meta.title,
                out.mentions.len()
            );
            for m in &out.mentions {
                println!("  mentions {prefix}/{m}");
            }
            Ok(())
        }
        Some("list") => {
            let prefix = args
                .get(1)
                .filter(|a| !a.starts_with("--"))
                .ok_or_else(|| usage.clone())?;
            let all = args.iter().any(|a| a == "--all");
            let store = open_store()?;
            let index = build_index(&store)?;
            let now = now_ms();
            let mut seen = BTreeSet::new();
            let mut any = false;
            let mut hidden = 0usize;
            for node in index.entities_by_kind(noun) {
                let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
                    continue;
                };
                if labels.get("prefix") != Some(prefix) || !seen.insert(id.clone()) {
                    continue;
                }
                let (lc, _) =
                    brain_observe::lifecycle::of(&index, &store, &id).map_err(|e| e.to_string())?;
                if !lc.is_active() && !all {
                    hidden += 1;
                    continue;
                }
                let slug = labels.get("slug").cloned().unwrap_or_default();
                let title = brain_observe::twin::latest(&index, &store, &id, "title")
                    .map_err(|e| e.to_string())?
                    .or_else(|| labels.get("title").cloned())
                    .unwrap_or_else(|| slug.clone());
                let status = brain_observe::twin::latest(&index, &store, &id, "status")
                    .map_err(|e| e.to_string())?
                    .map(|s| format!("[{s}] "))
                    .unwrap_or_default();
                let age = brain_observe::twin::latest_at(&index, &store, &id, "content")
                    .map_err(|e| e.to_string())?
                    .map(|(at, _)| format!("{}s ago", now.saturating_sub(at) / 1000))
                    .unwrap_or_else(|| "?".to_string());
                let mentions = relation_targets(&store, &index, &id, "mentions")?.len();
                let tag = if lc.is_active() {
                    String::new()
                } else {
                    format!("  [{}]", lc.as_str())
                };
                println!("{status}{slug}: {title}  ({age}, {mentions} mention(s)){tag}");
                any = true;
            }
            if !any {
                println!(
                    "no {}{noun}s under {prefix}",
                    if all { "" } else { "active " }
                );
            }
            if hidden > 0 {
                println!("({hidden} non-active hidden — --all shows history)");
            }
            Ok(())
        }
        Some("show") => {
            let (prefix, slug) = match (args.get(1), args.get(2)) {
                (Some(p), Some(s)) => (p, s),
                _ => return Err(usage),
            };
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = brain_core::ids::StableId::derive(&[noun, prefix, slug]);
            let content = brain_observe::twin::latest(&index, &store, &sid, "content")
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("no {noun} '{slug}' under {prefix}"))?;
            print!("{content}");
            if !content.ends_with('\n') {
                println!();
            }
            let (lc, why) =
                brain_observe::lifecycle::of(&index, &store, &sid).map_err(|e| e.to_string())?;
            if !lc.is_active() {
                let detail = if why.is_empty() {
                    String::new()
                } else {
                    format!(" ({why})")
                };
                println!("--- lifecycle: {}{detail} ---", lc.as_str());
            }
            // Status timeline (decisions), oldest first.
            let mut statuses = Vec::new();
            for id in index.observations_of(&sid) {
                if let Object::Observation {
                    property,
                    value,
                    observed_at_ms,
                    ..
                } = store.get(&id).map_err(|e| e.to_string())?
                {
                    if property == "status" {
                        statuses.push((observed_at_ms, value));
                    }
                }
            }
            statuses.sort();
            if !statuses.is_empty() {
                println!("--- status timeline ---");
                for (at, s) in statuses {
                    println!("{at}  {s}");
                }
            }
            let mentions = relation_targets(&store, &index, &sid, "mentions")?;
            if !mentions.is_empty() {
                println!("--- mentions ---");
                for m in mentions {
                    println!("{}", entity_label(&store, &index, &m));
                }
            }
            Ok(())
        }
        _ => Err(usage),
    }
}

/// `brain skill ...` / `brain agentcfg ...` — the agent-operating layer.
fn cmd_agent_doc(args: &[String], kind: brain_observe::agents::AgentDocKind) -> Result<(), String> {
    use brain_observe::{agents, twin};
    let noun = kind.as_str();
    let cmd = match kind {
        agents::AgentDocKind::Skill => "skill",
        agents::AgentDocKind::Config => "agentcfg",
    };
    let usage = format!(
        "usage: brain {cmd} add <file> --prefix <p> [--agent A] [--role R] | \
         brain {cmd} list <prefix> | brain {cmd} show <prefix> <slug>"
    );
    match args.first().map(String::as_str) {
        Some("add") => {
            let mut file = None;
            let mut prefix = None;
            let mut agent = None;
            let mut role = None;
            let mut it = args[1..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--prefix" => prefix = it.next().cloned(),
                    "--agent" => agent = it.next().cloned(),
                    "--role" => role = it.next().cloned(),
                    other if file.is_none() && !other.starts_with("--") => {
                        file = Some(other.to_string())
                    }
                    other => return Err(format!("unexpected argument '{other}'\n{usage}")),
                }
            }
            let file = file.ok_or_else(|| usage.clone())?;
            let prefix = prefix.ok_or_else(|| format!("--prefix is required\n{usage}"))?;
            let content =
                std::fs::read_to_string(&file).map_err(|e| format!("cannot read '{file}': {e}"))?;
            // Detect slug/agent/role from the path conventions when they
            // apply, then let explicit flags override.
            let normalized = file.replace('\\', "/");
            let (slug, det_agent, det_role) = match agents::path_agent_doc(&normalized) {
                Some((k, slug, a, r)) if k == kind => (slug, a, r),
                _ => {
                    let stem = normalized.rsplit('/').next().unwrap_or(&normalized);
                    let stem = stem.strip_suffix(".md").unwrap_or(stem).to_lowercase();
                    let default_role = if cmd == "skill" {
                        "skill"
                    } else {
                        "instructions"
                    };
                    (stem, "generic".to_string(), default_role.to_string())
                }
            };
            let doc = agents::parse_agent_content(
                kind,
                &slug,
                agent.as_deref().unwrap_or(&det_agent),
                role.as_deref().unwrap_or(&det_role),
                &content,
            );
            let store = open_store()?;
            let out = twin::add_agent_doc(&store, &prefix, &doc, &content, "claude-code")
                .map_err(|e| e.to_string())?;
            let state = if out.wrote {
                "recorded"
            } else {
                "already recorded (unchanged)"
            };
            println!(
                "{noun} '{slug}' {state} under {prefix} (agent: {}, role: {}, {} mention(s))",
                doc.agent,
                doc.role,
                out.mentions.len()
            );
            for m in &out.mentions {
                println!("  mentions {prefix}/{m}");
            }
            Ok(())
        }
        Some("list") => {
            let prefix = args
                .get(1)
                .filter(|a| !a.starts_with("--"))
                .ok_or_else(|| usage.clone())?;
            let all = args.iter().any(|a| a == "--all");
            let store = open_store()?;
            let index = build_index(&store)?;
            let mut seen = BTreeSet::new();
            let mut any = false;
            let mut hidden = 0usize;
            for node in index.entities_by_kind(noun) {
                let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
                    continue;
                };
                if labels.get("prefix") != Some(prefix) || !seen.insert(id.clone()) {
                    continue;
                }
                let (lc, _) =
                    brain_observe::lifecycle::of(&index, &store, &id).map_err(|e| e.to_string())?;
                if !lc.is_active() && !all {
                    hidden += 1;
                    continue;
                }
                let slug = labels.get("slug").cloned().unwrap_or_default();
                let agent = brain_observe::twin::latest(&index, &store, &id, "agent")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_else(|| "generic".to_string());
                let role = brain_observe::twin::latest(&index, &store, &id, "role")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default();
                let desc = brain_observe::twin::latest(&index, &store, &id, "description")
                    .map_err(|e| e.to_string())?
                    .map(|d| format!("  — {d}"))
                    .unwrap_or_default();
                let tag = if lc.is_active() {
                    String::new()
                } else {
                    format!("  [{}]", lc.as_str())
                };
                println!("[{agent}] {slug} ({role}){desc}{tag}");
                any = true;
            }
            if !any {
                println!(
                    "no {}{noun}s under {prefix}",
                    if all { "" } else { "active " }
                );
            }
            if hidden > 0 {
                println!("({hidden} non-active hidden — --all shows history)");
            }
            Ok(())
        }
        Some("show") => {
            let (prefix, slug) = match (args.get(1), args.get(2)) {
                (Some(p), Some(s)) => (p, s),
                _ => return Err(usage),
            };
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = brain_core::ids::StableId::derive(&[noun, prefix, slug]);
            let content = brain_observe::twin::latest(&index, &store, &sid, "content")
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("no {noun} '{slug}' under {prefix}"))?;
            print!("{content}");
            if !content.ends_with('\n') {
                println!();
            }
            let mentions = relation_targets(&store, &index, &sid, "mentions")?;
            if !mentions.is_empty() {
                println!("--- mentions ---");
                for m in mentions {
                    println!("{}", entity_label(&store, &index, &m));
                }
            }
            Ok(())
        }
        _ => Err(usage),
    }
}

/// `brain template ...` — the deliverable contract, defined in the graph.
fn cmd_template(args: &[String]) -> Result<(), String> {
    use brain_observe::{templates, twin};
    let usage = "usage: brain template seed | list | show <slug> | \
                 set <slug> [--applies-to k] [--capture \"globs\"] [--fields \"spec\"] \
                 [--requires \"a,b\"] [--rot none|info|warn] [--placement P] [--enforce E] \
                 [--home \"globs\"] [--project-to path] [--parser p] [--links \"a,b\"] \
                 [--extensions \"txt,yaml\"] [--title T]";
    match args.first().map(String::as_str) {
        Some("set") => {
            let slug = args.get(1).filter(|s| !s.starts_with("--")).ok_or(usage)?;
            let mut props: Vec<(String, String)> = Vec::new();
            let mut title = None;
            let mut it = args[2..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--applies-to" | "--capture" | "--fields" | "--requires" | "--home"
                    | "--project-to" | "--links" | "--extensions" => {
                        let key = a.trim_start_matches("--").replace('-', "_");
                        let v = it.next().cloned().ok_or(format!("{a} needs a value"))?;
                        props.push((key, v));
                    }
                    "--rot" => {
                        let v = it.next().cloned().ok_or("--rot needs a value")?;
                        if !["none", "info", "warn"].contains(&v.as_str()) {
                            return Err(format!("--rot must be none|info|warn, got '{v}'"));
                        }
                        props.push(("rot".to_string(), v));
                    }
                    "--placement" => {
                        let v = it.next().cloned().ok_or("--placement needs a value")?;
                        if !["graph_first", "file_first", "projection"].contains(&v.as_str()) {
                            return Err(format!(
                                "--placement must be graph_first|file_first|projection, got '{v}'"
                            ));
                        }
                        props.push(("placement".to_string(), v));
                    }
                    "--enforce" => {
                        let v = it.next().cloned().ok_or("--enforce needs a value")?;
                        if !["advisory", "enforced"].contains(&v.as_str()) {
                            return Err(format!("--enforce must be advisory|enforced, got '{v}'"));
                        }
                        props.push(("enforce".to_string(), v));
                    }
                    "--parser" => {
                        let v = it.next().cloned().ok_or("--parser needs a value")?;
                        if !["doc.decision", "doc.plan", "agent", "fields"].contains(&v.as_str()) {
                            return Err(format!(
                                "--parser must be doc.decision|doc.plan|agent|fields, got '{v}'"
                            ));
                        }
                        props.push(("parser".to_string(), v));
                    }
                    "--title" => title = it.next().cloned(),
                    other => return Err(format!("unexpected argument '{other}'\n{usage}")),
                }
            }
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = templates::template_sid(slug);
            let mut labels = std::collections::BTreeMap::new();
            labels.insert("slug".to_string(), slug.to_string());
            labels.insert(
                "title".to_string(),
                title.clone().unwrap_or_else(|| slug.to_string()),
            );
            store
                .put(&Object::Entity {
                    id: sid.clone(),
                    entity_kind: "template".to_string(),
                    labels,
                })
                .map_err(|e| e.to_string())?;
            if let Some(t) = title {
                props.push(("title".to_string(), t));
            }
            let now = now_ms();
            let mut wrote = 0;
            for (prop, value) in &props {
                if twin::latest(&index, &store, &sid, prop)
                    .map_err(|e| e.to_string())?
                    .as_deref()
                    != Some(value.as_str())
                {
                    store
                        .put(&Object::Observation {
                            subject: sid.clone(),
                            property: prop.clone(),
                            value: value.clone(),
                            source: "agent".to_string(),
                            observed_at_ms: now,
                        })
                        .map_err(|e| e.to_string())?;
                    wrote += 1;
                }
            }
            // Version the contract: this-run values win over the index.
            let this_run = |key: &str| {
                props
                    .iter()
                    .rev()
                    .find(|(p, _)| p == key)
                    .map(|(_, v)| v.clone())
            };
            let requires = match this_run("requires") {
                Some(v) => v,
                None => twin::latest(&index, &store, &sid, "requires")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default(),
            };
            let content = twin::latest(&index, &store, &sid, "content")
                .map_err(|e| e.to_string())?
                .unwrap_or_default();
            wrote +=
                templates::stamp_contract(&store, &index, &sid, &requires, &content, "agent", now)
                    .map_err(|e| e.to_string())?;
            println!("template '{slug}': {wrote} observation(s) written");
            if props.iter().any(|(p, _)| p == "capture") {
                println!("the twin now auto-captures matching paths on every refresh");
            }
            Ok(())
        }
        Some("seed") => {
            let store = open_store()?;
            let n = templates::seed(&store).map_err(|e| e.to_string())?;
            println!(
                "{} templates present; {n} observation(s) written",
                templates::DEFAULTS.len()
            );
            Ok(())
        }
        Some("fitness") => {
            let slug = args
                .get(1)
                .filter(|a| !a.starts_with("--"))
                .map(String::as_str);
            let prefix = parse_prefix(&args[1..]);
            let store = open_store()?;
            let index = build_index(&store)?;
            let all = brain_observe::fitness::fitness(&store, &index, &prefix, slug)
                .map_err(|e| e.to_string())?;
            if all.is_empty() {
                println!("no fitness data yet — artifacts record their judging contract as they are captured");
                return Ok(());
            }
            for tf in &all {
                println!(
                    "template {} ({}, {}) — {} version(s) seen in use",
                    tf.slug,
                    tf.kind,
                    tf.enforce,
                    tf.versions.len()
                );
                for v in &tf.versions {
                    let cur = if v.current { ", current" } else { "" };
                    let (ok, total) = v.first_conform;
                    let pct = if total > 0 { ok * 100 / total } else { 100 };
                    println!(
                        "  version {}{cur}: {} artifact(s); first-capture conformance {ok}/{total} ({pct}%)",
                        &v.contract[..v.contract.len().min(12)],
                        v.artifacts
                    );
                    for (field, n) in &v.missing {
                        println!("    missing at first capture: {field} ×{n}");
                    }
                    let outcomes: Vec<String> =
                        v.outcomes.iter().map(|(s, n)| format!("{s} {n}")).collect();
                    println!(
                        "    outcomes: {}; currently stale: {}",
                        outcomes.join(", "),
                        v.stale_now
                    );
                }
                for verdict in &tf.verdicts {
                    println!("  verdict: {verdict}");
                }
            }
            Ok(())
        }
        Some("evolve") => {
            let slug = args.get(1).filter(|a| !a.starts_with("--")).ok_or(usage)?;
            let prefix = parse_prefix(&args[2..]);
            let apply = args.iter().any(|a| a == "--apply");
            let store = open_store()?;
            let index = build_index(&store)?;
            let Some(ev) = brain_observe::fitness::evolve(&store, &index, &prefix, slug)
                .map_err(|e| e.to_string())?
            else {
                println!("no evolution suggested for '{slug}' — the evidence is thin or the contract fits");
                return Ok(());
            };
            println!(
                "proposal for '{slug}': demote [{}] from requires (missed in ≥ half of first captures)",
                ev.demote.join(", ")
            );
            println!(
                "  new: brain template set {slug} --requires \"{}\"  (+ recommended: {})",
                ev.new_requires.join(","),
                ev.new_recommended.join(",")
            );
            if apply {
                brain_observe::fitness::apply_evolution(&store, &index, slug, &ev)
                    .map_err(|e| e.to_string())?;
                println!("applied — contract_b3 bumped; the next measurement window is open");
            } else {
                println!("(re-run with --apply to accept; old artifacts keep the version that judged them)");
            }
            Ok(())
        }
        Some("list") => {
            let store = open_store()?;
            let index = build_index(&store)?;
            for (applies, (sid, requires)) in
                templates::by_kind(&store, &index).map_err(|e| e.to_string())?
            {
                let title = twin::latest(&index, &store, &sid, "title")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default();
                let capture = twin::latest(&index, &store, &sid, "capture")
                    .map_err(|e| e.to_string())?
                    .map(|c| format!("  captures [{c}]"))
                    .unwrap_or_default();
                println!(
                    "{applies:<12} requires [{}]{capture}  — {title}",
                    requires.join(", ")
                );
            }
            Ok(())
        }
        Some("show") => {
            let slug = args.get(1).ok_or(usage)?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = templates::template_sid(slug);
            let content = twin::latest(&index, &store, &sid, "content")
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("no template '{slug}' (run: brain template seed)"))?;
            print!("{content}");
            Ok(())
        }
        _ => Err(usage.to_string()),
    }
}

/// Render every graph-first artifact's projection, plus pure-projection
/// kinds (the capability matrix — a rendered query, never authored).
fn render_projections(
    store: &Store,
    index: &MemIndex,
    root: &std::path::Path,
    prefix: &str,
    only_kind: Option<&str>,
) -> Result<usize, String> {
    use brain_observe::{features, projection, twin};
    let reg = brain_observe::kinds::registry(store, index).map_err(|e| e.to_string())?;
    let mut rendered = 0usize;
    for (kind, def) in &reg {
        if only_kind.is_some_and(|k| k != kind) || def.project_to.is_empty() {
            continue;
        }
        if def.placement == "graph_first" {
            let mut seen = BTreeSet::new();
            for node in index.entities_by_kind(kind) {
                let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
                    continue;
                };
                if labels.get("prefix").map(String::as_str) != Some(prefix)
                    || !seen.insert(id.clone())
                {
                    continue;
                }
                if !brain_observe::lifecycle::of(index, store, &id)
                    .map_err(|e| e.to_string())?
                    .0
                    .is_active()
                {
                    continue;
                }
                let slug = labels.get("slug").cloned().unwrap_or_default();
                let Some(content) =
                    twin::latest(index, store, &id, "content").map_err(|e| e.to_string())?
                else {
                    continue;
                };
                let Some(rel) = projection::projection_rel(def, &slug) else {
                    continue;
                };
                let body = projection::render_body(&rel, kind, prefix, &slug, &content);
                projection::write_projection(store, index, root, &id, &rel, &body)
                    .map_err(|e| e.to_string())?;
                rendered += 1;
            }
        } else if def.placement == "projection" && kind == "capability_matrix" {
            let rows = features::list(store, index, prefix).map_err(|e| e.to_string())?;
            if rows.is_empty() {
                continue;
            }
            let mut content = String::from(
                "# Capability matrix\n\nA rendered query over the feature registry — regenerate, never edit.\n\n| feature | status | definition of done |\n|---|---|---|\n",
            );
            for row in &rows {
                let report = features::evaluate(store, index, prefix, &row.slug)
                    .map_err(|e| e.to_string())?;
                let met = report.checks.iter().filter(|c| c.count > 0).count();
                let mark = if report.done { " ✓" } else { "" };
                content.push_str(&format!(
                    "| {} | {} | {met}/{}{mark} |\n",
                    row.slug,
                    row.status,
                    report.checks.len()
                ));
            }
            let slug = "features";
            let sid = brain_core::ids::StableId::derive(&["capability_matrix", prefix, slug]);
            let mut labels = std::collections::BTreeMap::new();
            labels.insert("prefix".to_string(), prefix.to_string());
            labels.insert("slug".to_string(), slug.to_string());
            labels.insert("title".to_string(), "Capability matrix".to_string());
            store
                .put(&Object::Entity {
                    id: sid.clone(),
                    entity_kind: "capability_matrix".to_string(),
                    labels,
                })
                .map_err(|e| e.to_string())?;
            if twin::latest(index, store, &sid, "content")
                .map_err(|e| e.to_string())?
                .as_deref()
                != Some(content.as_str())
            {
                store
                    .put(&Object::Observation {
                        subject: sid.clone(),
                        property: "content".to_string(),
                        value: content.clone(),
                        source: "projection".to_string(),
                        observed_at_ms: now_ms(),
                    })
                    .map_err(|e| e.to_string())?;
            }
            let Some(rel) = projection::projection_rel(def, slug) else {
                continue;
            };
            let body = projection::render_body(&rel, kind, prefix, slug, &content);
            projection::write_projection(store, index, root, &sid, &rel, &body)
                .map_err(|e| e.to_string())?;
            rendered += 1;
        }
    }
    Ok(rendered)
}

/// `brain artifact ...` — generic browse for artifacts of any entity kind,
/// including kinds taught to the store via graph-defined capture rules.
fn cmd_artifact(args: &[String]) -> Result<(), String> {
    let usage = "usage: brain artifact new|edit <prefix> <kind> <slug> [--title T] [--file f|-] | \
                 brain artifact render [dir] [--prefix <p>] [--kind k] [--check] | \
                 brain artifact list <prefix> <kind> [--all] | \
                 brain artifact show <prefix> <kind> <slug> | \
                 brain artifact set-lifecycle <prefix> <kind> <slug> <state> [--why R] | \
                 brain artifact ack <prefix> <kind> <slug> [--note T]";
    match args.first().map(String::as_str) {
        Some(op @ ("new" | "edit")) => {
            let (prefix, kind, slug) = match (args.get(1), args.get(2), args.get(3)) {
                (Some(p), Some(k), Some(s))
                    if !p.starts_with("--") && !k.starts_with("--") && !s.starts_with("--") =>
                {
                    (p, k, s)
                }
                _ => return Err(usage.to_string()),
            };
            let mut title = None;
            let mut file = None;
            let mut it = args[4..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--title" => title = it.next().cloned(),
                    "--file" => file = it.next().cloned(),
                    other => return Err(format!("unexpected argument '{other}'\n{usage}")),
                }
            }
            let store = open_store()?;
            let index = build_index(&store)?;
            let reg = brain_observe::kinds::registry(&store, &index).map_err(|e| e.to_string())?;
            let def = reg
                .get(kind.as_str())
                .ok_or_else(|| format!("unknown kind '{kind}' — teach it: brain template set <slug> --applies-to {kind} ..."))?
                .clone();
            let sid = brain_core::ids::StableId::derive(&[kind, prefix, slug]);
            let exists = !index.entity_nodes(&sid).is_empty();
            if op == "edit" && !exists {
                return Err(format!(
                    "no {kind} '{slug}' under {prefix} — use artifact new"
                ));
            }
            let title = title.unwrap_or_else(|| slug.replace('-', " "));
            let content = match file.as_deref() {
                Some("-") => {
                    use std::io::Read as _;
                    let mut buf = String::new();
                    std::io::stdin()
                        .read_to_string(&mut buf)
                        .map_err(|e| e.to_string())?;
                    buf
                }
                Some(path) => std::fs::read_to_string(path)
                    .map_err(|e| format!("cannot read '{path}': {e}"))?,
                None if op == "new" && !def.content.is_empty() => {
                    brain_observe::templates::instantiate(&def.content, &title)
                }
                None => return Err(format!("--file <path|-> is required\n{usage}")),
            };

            // The write-time gate: validate before anything lands.
            let missing = brain_observe::templates::check(&content, &def.requires);
            if !missing.is_empty() {
                let fix = format!(
                    "missing: {} — scaffold: brain deliverable new {} --title \"{title}\"",
                    missing.join(", "),
                    def.slug
                );
                if def.enforce == "enforced" {
                    return Err(format!(
                        "refused: {kind} '{slug}' does not meet its contract; nothing written\n  {fix}"
                    ));
                }
                eprintln!(
                    "warning: {kind} '{slug}' does not meet its contract ({fix}) — recorded anyway"
                );
            }
            let out = brain_observe::twin::author_artifact(
                &store, prefix, kind, slug, &title, &content, "agent",
            )
            .map_err(|e| e.to_string())?;
            let state = if out.wrote {
                "recorded"
            } else {
                "already recorded (unchanged)"
            };
            println!(
                "{kind} '{slug}' {state} under {prefix} ({} mention(s))",
                out.mentions.len()
            );

            // Graph-first kinds materialize as read-only projections.
            if def.placement == "graph_first" {
                if let Some(rel) = brain_observe::projection::projection_rel(&def, slug) {
                    let index = build_index(&store)?;
                    let body =
                        brain_observe::projection::render_body(&rel, kind, prefix, slug, &content);
                    let target = brain_observe::projection::write_projection(
                        &store,
                        &index,
                        std::path::Path::new("."),
                        &out.sid,
                        &rel,
                        &body,
                    )
                    .map_err(|e| e.to_string())?;
                    println!("projected (read-only): {}", target.display());
                }
            }
            Ok(())
        }
        Some("render") => {
            let dir = args
                .get(1)
                .filter(|a| !a.starts_with("--"))
                .cloned()
                .unwrap_or_else(|| ".".to_string());
            let prefix = parse_prefix(&args[1..]);
            let only_kind = args
                .iter()
                .position(|a| a == "--kind")
                .and_then(|i| args.get(i + 1))
                .cloned();
            let check = args.iter().any(|a| a == "--check");
            let store = open_store()?;
            let index = build_index(&store)?;
            let root = std::path::Path::new(&dir);
            if check {
                let found = brain_observe::projection::drift(&store, &index, root, &prefix)
                    .map_err(|e| e.to_string())?;
                if found.is_empty() {
                    println!("all projections match their contracts");
                    return Ok(());
                }
                for d in &found {
                    println!("{:?} {} — fix: {}", d.kind, d.path, d.fix);
                }
                if found
                    .iter()
                    .any(|d| d.kind == brain_observe::projection::DriftKind::HandEdited)
                {
                    return Err(format!(
                        "refused: {} projection(s) drifted from the graph",
                        found.len()
                    ));
                }
                return Ok(());
            }
            let rendered = render_projections(&store, &index, root, &prefix, only_kind.as_deref())?;
            println!(
                "{rendered} projection(s) rendered under {}/docs/brain",
                dir.trim_end_matches('/')
            );
            Ok(())
        }
        Some("ack") => {
            let (prefix, kind, slug) = match (args.get(1), args.get(2), args.get(3)) {
                (Some(p), Some(k), Some(s)) => (p, k, s),
                _ => return Err(usage.to_string()),
            };
            let note = args
                .iter()
                .position(|a| a == "--note")
                .and_then(|i| args.get(i + 1))
                .map(String::as_str)
                .unwrap_or("reviewed, still accurate");
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = brain_core::ids::StableId::derive(&[kind, prefix, slug]);
            if index.entity_nodes(&sid).is_empty() {
                return Err(format!("no {kind} '{slug}' under {prefix}"));
            }
            brain_observe::twin::ack(&store, &sid, note).map_err(|e| e.to_string())?;
            println!("{kind} '{slug}' acknowledged — staleness clock reset");
            Ok(())
        }
        Some("set-lifecycle") => {
            let (prefix, kind, slug, state) =
                match (args.get(1), args.get(2), args.get(3), args.get(4)) {
                    (Some(p), Some(k), Some(s), Some(st)) => (p, k, s, st),
                    _ => return Err(usage.to_string()),
                };
            let state = brain_observe::lifecycle::Lifecycle::parse(state).ok_or_else(|| {
                format!("unknown state '{state}' (active|done|abandoned|retired|superseded)")
            })?;
            let why = args
                .iter()
                .position(|a| a == "--why")
                .and_then(|i| args.get(i + 1))
                .map(String::as_str);
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = brain_core::ids::StableId::derive(&[kind, prefix, slug]);
            if index.entity_nodes(&sid).is_empty() {
                return Err(format!("no {kind} '{slug}' under {prefix}"));
            }
            let wrote = brain_observe::lifecycle::set(&store, &index, &sid, state, why)
                .map_err(|e| e.to_string())?;
            let verb = if wrote { "now" } else { "already" };
            println!("{kind} '{slug}' {verb} {}", state.as_str());
            Ok(())
        }
        Some("list") => {
            let (prefix, kind) = match (args.get(1), args.get(2)) {
                (Some(p), Some(k)) => (p, k),
                _ => return Err(usage.to_string()),
            };
            let all = args.iter().any(|a| a == "--all");
            let store = open_store()?;
            let index = build_index(&store)?;
            let mut seen = BTreeSet::new();
            let mut any = false;
            let mut hidden = 0usize;
            for node in index.entities_by_kind(kind) {
                let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
                    continue;
                };
                if labels.get("prefix") != Some(prefix) || !seen.insert(id.clone()) {
                    continue;
                }
                let (lc, _) =
                    brain_observe::lifecycle::of(&index, &store, &id).map_err(|e| e.to_string())?;
                if !lc.is_active() && !all {
                    hidden += 1;
                    continue;
                }
                let slug = labels.get("slug").cloned().unwrap_or_default();
                let title = brain_observe::twin::latest(&index, &store, &id, "title")
                    .map_err(|e| e.to_string())?
                    .or_else(|| labels.get("title").cloned())
                    .unwrap_or_else(|| slug.clone());
                let conforms = brain_observe::twin::latest(&index, &store, &id, "conforms")
                    .map_err(|e| e.to_string())?
                    .map(|c| {
                        if c == "true" {
                            "".to_string()
                        } else {
                            "  [nonconforming]".to_string()
                        }
                    })
                    .unwrap_or_default();
                let mentions = relation_targets(&store, &index, &id, "mentions")?.len();
                let tag = if lc.is_active() {
                    String::new()
                } else {
                    format!("  [{}]", lc.as_str())
                };
                println!("{slug}: {title}  ({mentions} mention(s)){conforms}{tag}");
                any = true;
            }
            if !any {
                println!(
                    "no {}{kind} artifacts under {prefix}",
                    if all { "" } else { "active " }
                );
            }
            if hidden > 0 {
                println!("({hidden} non-active hidden — --all shows history)");
            }
            Ok(())
        }
        Some("show") => {
            let (prefix, kind, slug) = match (args.get(1), args.get(2), args.get(3)) {
                (Some(p), Some(k), Some(s)) => (p, k, s),
                _ => return Err(usage.to_string()),
            };
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = brain_core::ids::StableId::derive(&[kind, prefix, slug]);
            // Latest value per property, so extracted fields show as a header.
            let mut latest_props: std::collections::BTreeMap<String, (u64, String)> =
                Default::default();
            for id in index.observations_of(&sid) {
                if let Object::Observation {
                    property,
                    value,
                    observed_at_ms,
                    ..
                } = store.get(&id).map_err(|e| e.to_string())?
                {
                    let entry = latest_props.entry(property).or_insert((0, String::new()));
                    if observed_at_ms >= entry.0 {
                        *entry = (observed_at_ms, value);
                    }
                }
            }
            if latest_props.is_empty() {
                return Err(format!("no {kind} '{slug}' under {prefix}"));
            }
            for (prop, (_, value)) in &latest_props {
                if prop != "content" {
                    println!("{prop}: {value}");
                }
            }
            if let Some((_, content)) = latest_props.get("content") {
                println!("---");
                print!("{content}");
                if !content.ends_with('\n') {
                    println!();
                }
            }
            let mentions = relation_targets(&store, &index, &sid, "mentions")?;
            if !mentions.is_empty() {
                println!("--- mentions ---");
                for m in mentions {
                    println!("{}", entity_label(&store, &index, &m));
                }
            }
            Ok(())
        }
        _ => Err(usage.to_string()),
    }
}

/// `brain tidy` — the brain cleans up: advisory scan, safe fixes via
/// governed changes, deletion only by explicit `--rm`.
fn cmd_tidy(args: &[String]) -> Result<(), String> {
    let dir = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| ".".to_string());
    let prefix = parse_prefix(args);
    let apply_fix = args.iter().any(|a| a == "--fix");
    let caps: Vec<String> = args
        .iter()
        .enumerate()
        .filter(|(_, a)| *a == "--cap")
        .filter_map(|(i, _)| args.get(i + 1).cloned())
        .collect();
    let rm = args
        .iter()
        .position(|a| a == "--rm")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let store = open_store()?;
    let index = build_index(&store)?;
    let root = std::path::Path::new(&dir);

    if let Some(path) = rm {
        let ok = brain_observe::tidy::remove_path(&store, root, &path, &caps)
            .map_err(|e| e.to_string())?;
        if ok {
            println!("removed {path} (intent + receipt recorded)");
        } else {
            return Err(format!("removal of {path} failed — see the receipt"));
        }
        return Ok(());
    }

    let findings =
        brain_observe::tidy::scan(&store, &index, root, &prefix).map_err(|e| e.to_string())?;
    if findings.is_empty() {
        println!("nothing to tidy under {prefix}");
        return Ok(());
    }
    for f in &findings {
        println!(
            "[{}] {} — {}\n    fix: {}",
            f.category, f.path, f.detail, f.fix
        );
    }
    if !apply_fix {
        let fixable = findings.iter().filter(|f| f.fixable).count();
        println!(
            "({} finding(s), {fixable} fixable — `brain tidy {dir} --prefix {prefix} --fix --cap fs`; deletion only via --rm <path>)",
            findings.len()
        );
        return Ok(());
    }
    let (fixed, skipped) = brain_observe::tidy::fix(&store, root, &prefix, &findings, &caps)
        .map_err(|e| e.to_string())?;
    for m in &fixed {
        println!("fixed: {m}");
    }
    for (path, why) in &skipped {
        println!("skipped: {path} — {why}");
    }
    println!("({} fixed, {} skipped)", fixed.len(), skipped.len());
    Ok(())
}

/// `brain instructions generate` — one guardrail block, every agent file.
fn cmd_instructions(args: &[String]) -> Result<(), String> {
    let usage = "usage: brain instructions generate [dir] [--prefix <p>] [--check]";
    if args.first().map(String::as_str) != Some("generate") {
        return Err(usage.to_string());
    }
    let dir = args
        .get(1)
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| ".".to_string());
    let prefix = parse_prefix(&args[1..]);
    let check = args.iter().any(|a| a == "--check");
    let store = open_store()?;
    let index = build_index(&store)?;
    let root = std::path::Path::new(&dir);
    if check {
        let drifted = brain_observe::instructions::block_drift(&store, &index, root, &prefix)
            .map_err(|e| e.to_string())?;
        if drifted.is_empty() {
            println!("instruction blocks match the registry");
            return Ok(());
        }
        return Err(format!(
            "instruction block out of date in: {} — run `brain instructions generate {dir} --prefix {prefix}`",
            drifted.join(", ")
        ));
    }
    for (file, changed) in brain_observe::instructions::generate(&store, &index, root, &prefix)
        .map_err(|e| e.to_string())?
    {
        let state = if changed { "updated" } else { "unchanged" };
        println!("{file}: guardrail block {state}");
    }
    println!("every agent family now reads identical rules; regenerate after `brain template set`");
    Ok(())
}

/// `brain asset ...` — typed binary artifacts: bytes stay files, identity,
/// ownership, and staleness live in the graph.
fn cmd_asset(args: &[String]) -> Result<(), String> {
    let usage = "usage: brain asset add <file> --prefix <p> --for <kind>/<slug> \
                 [--depicts <path|kind/slug>]... [--subtype s] | brain asset list <prefix> [--all]";
    match args.first().map(String::as_str) {
        Some("add") => {
            let file = args.get(1).filter(|a| !a.starts_with("--")).ok_or(usage)?;
            let mut prefix = None;
            let mut owner = None;
            let mut subtype = None;
            let mut depicts: Vec<String> = Vec::new();
            let mut it = args[2..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--prefix" => prefix = it.next().cloned(),
                    "--for" => owner = it.next().cloned(),
                    "--subtype" => subtype = it.next().cloned(),
                    "--depicts" => {
                        depicts.push(it.next().cloned().ok_or("--depicts needs a value")?)
                    }
                    other => return Err(format!("unexpected argument '{other}'\n{usage}")),
                }
            }
            let prefix = prefix.ok_or_else(|| format!("--prefix is required\n{usage}"))?;
            let owner = owner.ok_or_else(|| format!("--for <kind>/<slug> is required\n{usage}"))?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let rel = file.trim_start_matches("./");
            if store
                .resolve(&format!("{prefix}/{rel}"))
                .map_err(|e| e.to_string())?
                .is_none()
            {
                return Err(format!(
                    "'{rel}' is not twinned under {prefix} — run `brain twin refresh` first"
                ));
            }
            let (okind, oslug) = owner
                .split_once('/')
                .ok_or("--for takes <kind>/<slug>, e.g. plan/twin-v3")?;
            let owner_sid = brain_core::ids::StableId::derive(&[okind, &prefix, oslug]);
            if index.entity_nodes(&owner_sid).is_empty() {
                return Err(format!("no {okind} '{oslug}' under {prefix}"));
            }
            let mut targets = Vec::new();
            for d in &depicts {
                let sid = brain_observe::assets::resolve_depicts(&store, &index, &prefix, d)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("--depicts '{d}' matches no twinned entity"))?;
                targets.push(sid);
            }
            let out = brain_observe::assets::add(
                &store,
                &prefix,
                rel,
                &owner_sid,
                &targets,
                subtype.as_deref(),
            )
            .map_err(|e| e.to_string())?;
            let state = if out.wrote {
                "declared"
            } else {
                "already declared (unchanged)"
            };
            println!(
                "asset '{}' {state} under {prefix} (owner: {owner}, {} depicts)",
                out.slug,
                targets.len()
            );
            Ok(())
        }
        Some("list") => {
            let prefix = args.get(1).filter(|a| !a.starts_with("--")).ok_or(usage)?;
            let all = args.iter().any(|a| a == "--all");
            let store = open_store()?;
            let index = build_index(&store)?;
            let rows =
                brain_observe::assets::list(&store, &index, prefix).map_err(|e| e.to_string())?;
            let mut any = false;
            let mut hidden = 0usize;
            for row in rows {
                if !row.lifecycle.is_active() && !all {
                    hidden += 1;
                    continue;
                }
                let owner = row.owner.map(|o| format!("  -> {o}")).unwrap_or_default();
                let tag = if row.lifecycle.is_active() {
                    String::new()
                } else {
                    format!("  [{}]", row.lifecycle.as_str())
                };
                println!("[{}] {} ({}){owner}{tag}", row.subtype, row.slug, row.path);
                any = true;
            }
            if !any {
                println!(
                    "no {}assets under {prefix}",
                    if all { "" } else { "active " }
                );
            }
            if hidden > 0 {
                println!("({hidden} non-active hidden — --all shows history)");
            }
            Ok(())
        }
        _ => Err(usage.to_string()),
    }
}

/// `brain deliverable new <template>` — instantiate a scaffold to stdout.
fn cmd_deliverable(args: &[String]) -> Result<(), String> {
    use brain_observe::{templates, twin};
    let usage = "usage: brain deliverable new <template-slug> [--title T]";
    if args.first().map(String::as_str) != Some("new") {
        return Err(usage.to_string());
    }
    let slug = args.get(1).ok_or(usage)?;
    let mut title = "Untitled".to_string();
    let mut it = args[2..].iter();
    while let Some(a) = it.next() {
        if a == "--title" {
            if let Some(t) = it.next() {
                title = t.clone();
            }
        }
    }
    let store = open_store()?;
    let index = build_index(&store)?;
    let sid = templates::template_sid(slug);
    let content = twin::latest(&index, &store, &sid, "content")
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no template '{slug}' (run: brain template seed)"))?;
    print!("{}", templates::instantiate(&content, &title));
    Ok(())
}

/// One line of a feature tree: readiness, then the score in its own terms.
fn print_part(title: &str, slug: &str, done: bool, score: (usize, usize), lead: &str) {
    let (met, total) = score;
    let mark = if done { "✓" } else { "·" };
    let tally = if total == 0 {
        "nothing to check".to_string()
    } else {
        format!("{met}/{total}")
    };
    println!("{lead}{mark} {title}  ({slug})  {tally}");
}

/// The parts under a feature, drawn with box characters so depth reads.
fn print_parts(parts: &[brain_observe::features::PartReport], lead: &str) {
    for (index, part) in parts.iter().enumerate() {
        let last = index + 1 == parts.len();
        let branch = if last { "└ " } else { "├ " };
        print_part(
            &part.title,
            &part.slug,
            part.done,
            (part.met, part.total),
            &format!("{lead}{branch}"),
        );
        let deeper = format!("{lead}{}", if last { "  " } else { "│ " });
        print_parts(&part.parts, &deeper);
    }
}

/// `brain feature ...` — the registry: features as entities, links as edges.
fn cmd_feature(args: &[String]) -> Result<(), String> {
    use brain_observe::features;
    let usage = "usage: brain feature add <prefix> <slug> [--title T] [--status S] [--part-of <parent>] | \
                 feature link <prefix> <slug> <predicate> <target> [--kind k] | \
                 feature list <prefix> | feature matrix <prefix> | feature tree <prefix> [slug]";
    match args.first().map(String::as_str) {
        Some("add") => {
            let (prefix, slug) = match (args.get(1), args.get(2)) {
                (Some(p), Some(s)) => (p, s),
                _ => return Err(usage.to_string()),
            };
            let mut title = slug.clone();
            let mut status = "planned".to_string();
            let mut part_of: Option<String> = None;
            let mut it = args[3..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--title" => title = it.next().cloned().unwrap_or(title),
                    "--status" => status = it.next().cloned().unwrap_or(status),
                    "--part-of" => part_of = it.next().cloned(),
                    other => return Err(format!("unexpected argument '{other}'\n{usage}")),
                }
            }
            let store = open_store()?;
            let (_, wrote) =
                features::add(&store, prefix, slug, &title, &status).map_err(|e| e.to_string())?;
            let state = if wrote { "recorded" } else { "unchanged" };
            println!("feature '{slug}' {state} under {prefix} (status: {status})");

            // Creating a part and attaching it is one act.
            if let Some(parent) = part_of {
                let index = build_index(&store)?;
                let (parent_sid, _) =
                    features::resolve_target_as(&store, &index, prefix, &parent, Some("feature"))
                        .map_err(|e| e.to_string())?
                        .ok_or_else(|| {
                            format!("no feature '{parent}' under {prefix} — register it first")
                        })?;
                let linked = features::link(&store, prefix, slug, features::PART_OF, &parent_sid)
                    .map_err(|e| e.to_string())?;
                println!(
                    "  {} part of '{parent}'",
                    if linked { "now" } else { "already" }
                );
            }
            Ok(())
        }
        Some("tree") => {
            let prefix = args.get(1).ok_or(usage)?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let only = args.get(2).filter(|a| !a.starts_with("--"));

            // Roots are features nothing else claims as a parent, unless
            // one was named.
            let mut roots: Vec<String> = Vec::new();
            for row in features::list(&store, &index, prefix).map_err(|e| e.to_string())? {
                if let Some(want) = only {
                    if row.slug == **want {
                        roots.push(row.slug);
                    }
                    continue;
                }
                let sid = features::feature_sid(prefix, &row.slug);
                if features::parent(&store, &index, &sid)
                    .map_err(|e| e.to_string())?
                    .is_none()
                {
                    roots.push(row.slug);
                }
            }
            if roots.is_empty() {
                println!("no features under {prefix}");
                return Ok(());
            }
            for slug in roots {
                let report =
                    features::evaluate(&store, &index, prefix, &slug).map_err(|e| e.to_string())?;
                let title = features::list(&store, &index, prefix)
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .find(|r| r.slug == slug)
                    .map(|r| r.title)
                    .unwrap_or_else(|| slug.clone());
                print_part(&title, &slug, report.done, report.score(), "");
                print_parts(&report.parts, "");
                if let Some(blocking) = &report.blocked_by {
                    println!("  waiting on: {blocking}");
                }
            }
            Ok(())
        }
        Some("link") => {
            let pos = positional(&args[1..]);
            let (prefix, slug, predicate, target) = match pos.as_slice() {
                [p, s, pr, t] => (*p, *s, *pr, *t),
                _ => return Err(usage.to_string()),
            };
            // A composition edge must land on a feature; otherwise a part
            // named like an ADR would silently attach to the ADR.
            let mut want = args
                .iter()
                .position(|a| a == "--kind")
                .and_then(|i| args.get(i + 1))
                .map(String::as_str);
            if want.is_none() && predicate == features::PART_OF {
                want = Some("feature");
            }
            let store = open_store()?;
            let index = build_index(&store)?;
            let (target_sid, kind) =
                features::resolve_target_as(&store, &index, prefix, target, want)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| match want {
                        Some(k) => format!("no {k} '{target}' under {prefix}"),
                        None => format!("no twinned entity matches '{target}' (file path, or the slug of any registered kind)"),
                    })?;
            // Advisory link vocabulary: warn (never refuse) when the
            // feature kind declares allowed predicates and this one is not
            // among them.
            let reg = brain_observe::kinds::registry(&store, &index).map_err(|e| e.to_string())?;
            if let Some(def) = reg.get("feature") {
                if !def.links.is_empty() && !def.links.contains(predicate) {
                    eprintln!(
                        "warning: '{predicate}' is not in the feature kind's link vocabulary [{}] — linked anyway",
                        def.links.join(", ")
                    );
                }
            }
            let wrote = features::link(&store, prefix, slug, predicate, &target_sid)
                .map_err(|e| e.to_string())?;
            let state = if wrote { "linked" } else { "already linked" };
            println!("{slug} -{predicate}-> {target} ({kind}): {state}");
            Ok(())
        }
        Some("list") => {
            let prefix = args.get(1).ok_or(usage)?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let rows = features::list(&store, &index, prefix).map_err(|e| e.to_string())?;
            if rows.is_empty() {
                println!("no features under {prefix}");
            }
            for row in rows {
                let report = features::evaluate(&store, &index, prefix, &row.slug)
                    .map_err(|e| e.to_string())?;
                let (met, total) = report.score();
                let done = if report.done { " ✓ done" } else { "" };
                let sid = features::feature_sid(prefix, &row.slug);
                let under = features::parent(&store, &index, &sid)
                    .map_err(|e| e.to_string())?
                    .map(|(_, parent)| format!("  part of {parent}"))
                    .unwrap_or_default();
                let terms = if report.by_parts() { "parts" } else { "linked" };
                println!(
                    "[{}] {}: {}  ({met}/{total} {terms}{done}){under}",
                    row.status, row.slug, row.title,
                );
            }
            Ok(())
        }
        Some("matrix") => {
            let prefix = args.get(1).ok_or(usage)?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let dod = features::dod(&store, &index).map_err(|e| e.to_string())?;
            let rows = features::list(&store, &index, prefix).map_err(|e| e.to_string())?;
            if rows.is_empty() {
                println!("no features under {prefix}");
                return Ok(());
            }
            let width = rows.iter().map(|r| r.slug.len()).max().unwrap_or(8).max(8);
            let header: Vec<String> = dod.iter().map(|d| d.replace("_by", "")).collect();
            println!("{:<width$}  {}  done", "feature", header.join("  "));
            for row in rows {
                let report = features::evaluate(&store, &index, prefix, &row.slug)
                    .map_err(|e| e.to_string())?;
                let cells: Vec<String> = report
                    .checks
                    .iter()
                    .zip(&header)
                    .map(|(c, h)| {
                        let mark = if c.count > 0 { "✓" } else { "✗" };
                        format!("{mark:^w$}", w = h.len().max(1))
                    })
                    .collect();
                let done = if report.done { "✓" } else { "✗" };
                println!("{:<width$}  {}  {done}", row.slug, cells.join("  "));
            }
            Ok(())
        }
        _ => Err(usage.to_string()),
    }
}

/// `brain done <prefix> <slug>` — evaluate a feature against the DoD and
/// record the outcome as a guarded observation.
fn cmd_done(args: &[String]) -> Result<(), String> {
    use brain_observe::features;
    let (prefix, slug) = match (args.first(), args.get(1)) {
        (Some(p), Some(s)) => (p, s),
        _ => return Err("usage: brain done <prefix> <feature-slug>".to_string()),
    };
    let store = open_store()?;
    let index = build_index(&store)?;
    let report = features::evaluate(&store, &index, prefix, slug).map_err(|e| e.to_string())?;

    if report.by_parts() {
        // A feature with parts is judged by its parts; its own links are
        // still shown, but they are evidence, not the verdict.
        println!("judged by its {} part(s):", report.parts.len());
        print_parts(&report.parts, "");
        let linked = report.checks.iter().filter(|c| c.count > 0).count();
        if linked > 0 {
            println!(
                "(also linked directly: {})",
                report
                    .checks
                    .iter()
                    .filter(|c| c.count > 0)
                    .map(|c| c.predicate.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    } else {
        for check in &report.checks {
            let mark = if check.count > 0 { "✓" } else { "✗" };
            println!("{mark} {}  ({} link(s))", check.predicate, check.count);
        }
    }

    println!(
        "{}: {}",
        slug,
        if report.done { "DONE" } else { "not done" }
    );
    if let Some(blocking) = &report.blocked_by {
        println!("waiting on: {blocking}");
    }
    features::record_done(&store, &index, prefix, slug, &report).map_err(|e| e.to_string())?;
    Ok(())
}

/// `brain watch` — the continuous loop, built in: refresh + insights on an
/// interval, optionally regenerating docs each round. Replaces the shell
/// wrapper so the monolithic binary needs no scripts.
fn cmd_watch(args: &[String]) -> Result<(), String> {
    let mut dir = ".".to_string();
    let mut prefix = "twin/self".to_string();
    let mut interval = 60u64;
    let mut docs = false;
    let mut it = args.iter();
    let mut positional = false;
    while let Some(a) = it.next() {
        match a.as_str() {
            "--prefix" => prefix = it.next().cloned().ok_or("--prefix needs a value")?,
            "--interval" => {
                interval = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--interval needs seconds")?
            }
            "--docs" => docs = true,
            other if !other.starts_with("--") && !positional => {
                dir = other.to_string();
                positional = true;
            }
            other => return Err(format!("unexpected argument '{other}'")),
        }
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    loop {
        println!("--- watch: refresh {prefix} ---");
        for cmd in [
            vec!["twin", "refresh", dir.as_str(), "--prefix", prefix.as_str()],
            vec!["twin", "insights", prefix.as_str()],
        ] {
            let _ = std::process::Command::new(&exe).args(&cmd).status();
        }
        if docs {
            let _ = std::process::Command::new(&exe)
                .args([
                    "docs",
                    "generate",
                    dir.as_str(),
                    "--prefix",
                    prefix.as_str(),
                ])
                .status();
        }
        std::thread::sleep(std::time::Duration::from_secs(interval));
    }
}

/// `brain eyes` — the read-only human projection served from this binary.
fn cmd_eyes(args: &[String]) -> Result<(), String> {
    let mut config = brain_eyes::Config {
        store_root: std::env::var("BRAIN_STORE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from(".brain")),
        ..brain_eyes::Config::default()
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--prefix" => {
                config.prefix = it.next().cloned().ok_or("--prefix needs a value")?;
            }
            "--bind" => {
                config.bind = it.next().cloned().ok_or("--bind needs an address")?;
            }
            "--port" => {
                config.port = it
                    .next()
                    .and_then(|value| value.parse().ok())
                    .ok_or("--port needs a number from 0 to 65535")?;
            }
            // Where file-backed bodies are read from. Defaults to the
            // working directory, which is the repo the twin observes.
            "--root" => {
                config.content_root = it
                    .next()
                    .map(std::path::PathBuf::from)
                    .ok_or("--root needs a directory")?;
            }
            other => {
                return Err(format!(
                    "unexpected argument '{other}'\nusage: brain eyes [--prefix P] [--bind IP] [--port N] [--root DIR]"
                ));
            }
        }
    }
    brain_eyes::serve(config)
}

/// `brain bench index` — the earn-adoption gate: honest numbers, cold
/// reference replay vs cortex warm open, plus a real query mix, with
/// answers verified identical before any timing is trusted.
fn cmd_bench(args: &[String]) -> Result<(), String> {
    if args.first().map(String::as_str) != Some("index") {
        return Err("usage: brain bench index [--prefix <p>]".to_string());
    }
    let prefix = args
        .iter()
        .position(|a| a == "--prefix")
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_else(|| "twin/self".to_string());
    let store = open_store()?;
    let objects = store.count_objects().map_err(|e| e.to_string())?;

    let t0 = std::time::Instant::now();
    let cold = cortex::Cortex::open_ephemeral(&store).map_err(|e| e.to_string())?;
    let cold_time = t0.elapsed();

    // Ensure a checkpoint exists, then measure the warm path.
    cortex::Cortex::open(&store)
        .and_then(|g| g.checkpoint().map(|_| ()))
        .map_err(|e| e.to_string())?;
    let t1 = std::time::Instant::now();
    let warm = cortex::Cortex::open(&store).map_err(|e| e.to_string())?;
    let warm_time = t1.elapsed();

    // Correctness first: identical answers over real probes, or no bench.
    let mut sids = Vec::new();
    for (name, node) in store.namespace().map_err(|e| e.to_string())? {
        if name.strip_prefix(&format!("{prefix}/")).is_some() {
            if let Ok(Object::Entity { id, .. }) = store.get(&node) {
                sids.push(id);
            }
        }
    }
    if !cortex::answers_match(
        &*cold,
        &*warm,
        &[],
        &sids,
        &["source_file", "decision", "test_run"],
        &["imports", "contains", "mentions"],
    ) {
        return Err("backends disagree — cortex does not earn adoption".to_string());
    }

    // A real query mix over the warm index — against a *fresh* store.
    //
    // The store caches objects for the life of the process, so running
    // this on the store the cold replay just walked would measure a fully
    // warm cache and report a number no real command can achieve. A
    // command opens a checkpoint and then reads the objects it needs; this
    // reproduces that.
    let query_store = open_store()?;
    let query_index = cortex::Cortex::open(&query_store).map_err(|e| e.to_string())?;
    let t2 = std::time::Instant::now();
    let mut edges = 0usize;
    for sid in &sids {
        edges += query_index.relations_from(sid, "imports").len();
        edges += query_index.relations_to(sid, "imports").len();
        edges += query_index.relations_from(sid, "contains").len();
    }
    let ins = brain_observe::twin::insights_with(&query_store, &query_index, &prefix)
        .map_err(|e| e.to_string())?;
    let query_time = t2.elapsed();

    // And again on the same store, to show what the cache is worth.
    let t3 = std::time::Instant::now();
    let _ = brain_observe::twin::insights_with(&query_store, &query_index, &prefix)
        .map_err(|e| e.to_string())?;
    let requery_time = t3.elapsed();

    println!(
        "store: {objects} objects; probes: {} entities, {edges} edge answers",
        sids.len()
    );
    println!("cold replay (BRAIN_INDEX=mem behavior): {cold_time:?}");
    println!(
        "warm cortex open (delta {} event(s)):   {warm_time:?}  ({:.1}x faster)",
        warm.delta(),
        cold_time.as_secs_f64() / warm_time.as_secs_f64().max(1e-9)
    );
    println!("query mix (edges + full insights):      {query_time:?}  (cold store)");
    println!("the same query again (warm objects):    {requery_time:?}");
    println!(
        "answers: identical across backends ({} files in insights)",
        ins.files
    );
    Ok(())
}

/// Resolve a point in time: epoch ms, a relative `30m`/`2h`/`1d` ago, or
/// a git commit hash looked up in the repo entity's observation timeline.
fn resolve_when(store: &Store, index: &MemIndex, prefix: &str, when: &str) -> Result<u64, String> {
    if when.chars().all(|c| c.is_ascii_digit()) && when.len() >= 12 {
        return when.parse().map_err(|e| format!("bad timestamp: {e}"));
    }
    if let Some(unit) = when.chars().last().filter(|c| "smhd".contains(*c)) {
        if let Ok(n) = when[..when.len() - 1].parse::<u64>() {
            let secs = match unit {
                's' => n,
                'm' => n * 60,
                'h' => n * 3600,
                _ => n * 86_400,
            };
            return Ok(now_ms().saturating_sub(secs * 1000));
        }
    }
    // A commit hash (prefix): when the twin observed that commit as HEAD.
    let repo = brain_core::ids::StableId::derive(&["repo", prefix]);
    for id in index.observations_of(&repo) {
        if let Object::Observation {
            property,
            value,
            observed_at_ms,
            ..
        } = store.get(&id).map_err(|e| e.to_string())?
        {
            if property == "git_commit" && value.starts_with(when) {
                return Ok(observed_at_ms);
            }
        }
    }
    Err(format!(
        "cannot resolve '{when}' (epoch ms, 30m/2h/1d, or a twinned commit hash)"
    ))
}

/// `brain change ...` — governed mode: the motor system. Changes to
/// twinned software go through the intent/receipt boundary, with an
/// explicit capability — never ambient authority.
fn cmd_change(args: &[String]) -> Result<(), String> {
    use brain_observe::govern;
    let usage = "usage: brain change propose <prefix> <path> --from <file> [--reason R] [--dir d] | \
                 apply|revert <prefix> <slug> --cap fs [--dir d] | verify <prefix> <slug> [--dir d] | \
                 list <prefix> | show <prefix> <slug>";
    let flag = |key: &str| -> Option<String> {
        args.iter()
            .position(|a| a == key)
            .and_then(|i| args.get(i + 1).cloned())
    };
    let dir = flag("--dir").unwrap_or_else(|| ".".to_string());
    let root = std::path::Path::new(&dir);
    match args.first().map(String::as_str) {
        Some("propose") => {
            let (prefix, path) = match (args.get(1), args.get(2)) {
                (Some(p), Some(t)) if !p.starts_with("--") && !t.starts_with("--") => (p, t),
                _ => return Err(usage.to_string()),
            };
            let from = flag("--from").ok_or("propose needs --from <content-file>")?;
            let reason = flag("--reason").unwrap_or_else(|| "unstated".to_string());
            let content =
                std::fs::read_to_string(&from).map_err(|e| format!("cannot read '{from}': {e}"))?;
            let store = open_store()?;
            let p = govern::propose(&store, root, prefix, path, &content, &reason)
                .map_err(|e| e.to_string())?;
            let state = if p.wrote {
                "proposed"
            } else {
                "already proposed"
            };
            println!(
                "change '{}' {state}: {} -> {} ({path})",
                p.slug,
                p.before_b3.as_deref().map(|h| &h[..8]).unwrap_or("absent"),
                &p.after_b3[..8]
            );
            println!(
                "apply with: brain change apply {prefix} {} --cap fs",
                p.slug
            );
            Ok(())
        }
        Some(op @ ("apply" | "revert")) => {
            let (prefix, slug) = match (args.get(1), args.get(2)) {
                (Some(p), Some(s)) if !p.starts_with("--") && !s.starts_with("--") => (p, s),
                _ => return Err(usage.to_string()),
            };
            let caps: Vec<String> = args
                .iter()
                .enumerate()
                .filter(|(_, a)| *a == "--cap")
                .filter_map(|(i, _)| args.get(i + 1).cloned())
                .collect();
            let store = open_store()?;
            let done = if op == "apply" {
                govern::apply(&store, root, prefix, slug, &caps)
            } else {
                govern::revert(&store, root, prefix, slug, &caps)
            }
            .map_err(|e| e.to_string())?;
            println!(
                "{op} {}: intent {} -> receipt {} ({})",
                slug,
                done.intent,
                done.receipt,
                if done.ok { "ok" } else { "FAILED" }
            );
            if done.ok && op == "apply" {
                println!("verify with: brain change verify {prefix} {slug}");
            }
            Ok(())
        }
        Some("verify") => {
            let (prefix, slug) = match (args.get(1), args.get(2)) {
                (Some(p), Some(s)) => (p, s),
                _ => return Err(usage.to_string()),
            };
            let store = open_store()?;
            let v = govern::verify(&store, root, prefix, slug).map_err(|e| e.to_string())?;
            println!(
                "{slug}: {} ({}/{} passed)",
                if v.passed { "VERIFIED" } else { "BROKEN" },
                v.total - v.failed,
                v.total
            );
            Ok(())
        }
        Some("list") => {
            let prefix = args.get(1).ok_or(usage)?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let mut seen = BTreeSet::new();
            let mut any = false;
            for node in index.entities_by_kind("change") {
                let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
                    continue;
                };
                if labels.get("prefix") != Some(prefix) || !seen.insert(id.clone()) {
                    continue;
                }
                let slug = labels.get("slug").cloned().unwrap_or_default();
                let status = brain_observe::twin::latest(&index, &store, &id, "status")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default();
                let target = labels.get("target").cloned().unwrap_or_default();
                let reason = labels.get("title").cloned().unwrap_or_default();
                println!("[{status}] {slug}  {target}  — {reason}");
                any = true;
            }
            if !any {
                println!("no changes under {prefix}");
            }
            Ok(())
        }
        Some("show") => {
            let (prefix, slug) = match (args.get(1), args.get(2)) {
                (Some(p), Some(s)) => (p, s),
                _ => return Err(usage.to_string()),
            };
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = brain_observe::govern::change_sid(prefix, slug);
            for prop in [
                "status",
                "target",
                "reason",
                "before_b3",
                "after_b3",
                "intent",
            ] {
                if let Some(v) = brain_observe::twin::latest(&index, &store, &sid, prop)
                    .map_err(|e| e.to_string())?
                {
                    println!("{prop}: {v}");
                }
            }
            // Status timeline: the change's whole life, oldest first.
            let mut timeline = Vec::new();
            for id in index.observations_of(&sid) {
                if let Object::Observation {
                    property,
                    value,
                    observed_at_ms,
                    ..
                } = store.get(&id).map_err(|e| e.to_string())?
                {
                    if property == "status" {
                        timeline.push((observed_at_ms, value));
                    }
                }
            }
            timeline.sort();
            println!("--- status timeline ---");
            for (at, s) in timeline {
                println!("{at}  {s}");
            }
            if let Some(content) = brain_observe::twin::latest(&index, &store, &sid, "content")
                .map_err(|e| e.to_string())?
            {
                println!("--- proposed content ({} bytes) ---", content.len());
            }
            Ok(())
        }
        _ => Err(usage.to_string()),
    }
}

/// `brain attend` — the attention organ: what deserves attention now.
/// `brain wake <prefix>` — one command, the whole present.
fn cmd_wake(args: &[String]) -> Result<(), String> {
    let prefix = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("usage: brain wake <prefix> [--full]")?;
    let full = args.iter().any(|a| a == "--full");
    let store = open_store()?;
    let index = build_index(&store)?;
    let text =
        brain_observe::wake::wake(&store, &index, prefix, full).map_err(|e| e.to_string())?;
    println!("{text}");
    Ok(())
}

fn cmd_attend(args: &[String]) -> Result<(), String> {
    let prefix = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("usage: brain attend <prefix> [--top N]")?;
    let top = parse_top(args, 10)?;
    let store = open_store()?;
    let index = build_index(&store)?;
    let ranked =
        brain_observe::attention::attend(&store, &index, prefix).map_err(|e| e.to_string())?;
    if ranked.is_empty() {
        println!("nothing demands attention under {prefix}");
    }
    for (i, a) in ranked.iter().take(top).enumerate() {
        println!(
            "{:>2}. [{:>3}] {} ({})  — {}",
            i + 1,
            a.score,
            a.label,
            a.kind,
            a.reasons.join(", ")
        );
    }
    Ok(())
}

/// `brain spine` — what each feature reaches, and what nothing claims.
fn cmd_spine(args: &[String]) -> Result<(), String> {
    let prefix = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("usage: brain spine <prefix> [--unclaimed <kind>]")?;
    let want = args
        .iter()
        .position(|a| a == "--unclaimed")
        .and_then(|i| args.get(i + 1));
    let store = open_store()?;
    let index = build_index(&store)?;
    let spine = brain_observe::spine::build(&store, &index, prefix).map_err(|e| e.to_string())?;

    if !spine.asked() {
        println!("no feature under {prefix} declares anything yet");
        return Ok(());
    }

    if let Some(kind) = want {
        let rows = spine.unclaimed(kind);
        if rows.is_empty() {
            println!("every {kind} under {prefix} is claimed by a feature");
        }
        for sid in rows {
            println!("  {}", brain_observe::twin::sid_label(&index, &store, sid));
        }
        return Ok(());
    }

    for slug in spine.slugs().map(str::to_string).collect::<Vec<_>>() {
        let reach = spine.reach(&slug).expect("just listed");
        let counts: Vec<String> = reach
            .by_kind
            .iter()
            .map(|(kind, rows)| format!("{} {kind}", rows.len()))
            .collect();
        println!(
            "{slug}  {} file(s) declared -> {}",
            reach.files.len(),
            if counts.is_empty() {
                "nothing".to_string()
            } else {
                counts.join(", ")
            }
        );
    }
    let (claimed, total) = spine.claimed_total();
    println!("\ncoverage: {claimed} of {total} records are claimed by a feature");
    for row in spine.census() {
        println!("  {:<16} {}/{}", row.kind, row.claimed, row.total);
    }
    if !spine.uncorroborated().is_empty() {
        println!("\nlinked, but nothing observed corroborates it:");
        for row in spine.uncorroborated() {
            println!(
                "  {} [{}] ×{} — {}",
                row.slug,
                row.predicate,
                row.targets.len(),
                row.why
            );
        }
    }
    Ok(())
}

/// `brain sleep` — the consolidation organ: distill history into memory.
fn cmd_sleep(args: &[String]) -> Result<(), String> {
    let prefix = args.first().ok_or("usage: brain sleep <prefix>")?;
    let store = open_store()?;
    let report = brain_observe::sleep::sleep(&store, prefix).map_err(|e| e.to_string())?;
    if report.wrote {
        println!("slept: {}", report.summary);
    } else {
        println!("{}", report.summary);
    }
    Ok(())
}

/// `brain related` — the association organ: soft, derived, disposable.
fn cmd_related(args: &[String]) -> Result<(), String> {
    let name = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("usage: brain related <name> [--top N]")?;
    let top = parse_top(args, 10)?;
    let store = open_store()?;
    let index = build_index(&store)?;
    let sid = entity_sid(&store, name)?;
    // The prefix is the longest bound repo entity whose name prefixes ours
    // (twin/self/src/main.rs -> twin/self).
    let mut prefix = String::new();
    for (n, node) in store.namespace().map_err(|e| e.to_string())? {
        if name.starts_with(&format!("{n}/")) && n.len() > prefix.len() {
            if let Ok(Object::Entity { entity_kind, .. }) = store.get(&node) {
                if entity_kind == "repo" {
                    prefix = n;
                }
            }
        }
    }
    if prefix.is_empty() {
        return Err(format!("cannot find a twin prefix for '{name}'"));
    }
    let assoc = brain_observe::assoc::AssocIndex::build(&store, &index, &prefix)
        .map_err(|e| e.to_string())?;
    let related = assoc.related(&sid);
    if related.is_empty() {
        println!("no associations for {name} (yet — associations grow with history)");
    }
    for (label, score, reasons) in related.into_iter().take(top) {
        println!("[{score:>3}] {label}  — {}", reasons.join(", "));
    }
    Ok(())
}

fn parse_top(args: &[String], default: usize) -> Result<usize, String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--top" {
            return it
                .next()
                .and_then(|v| v.parse().ok())
                .ok_or("--top needs a number".into());
        }
    }
    Ok(default)
}

/// `brain sessions ...` — the coding agents that worked here.
fn cmd_sessions(args: &[String]) -> Result<(), String> {
    use brain_observe::sessions;
    let usage = "usage: brain sessions import [dir] [--prefix <p>] [--agent claude|codex] [--since <ms|30m|2h|7d>] | brain sessions list <prefix>";
    match args.first().map(String::as_str) {
        Some("import") => {
            let mut dir = ".".to_string();
            let mut prefix = "twin/self".to_string();
            let mut agent: Option<String> = None;
            let mut since = 0u64;
            let mut it = args[1..].iter();
            let mut positional_taken = false;
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--prefix" => prefix = it.next().cloned().unwrap_or(prefix),
                    "--agent" => agent = it.next().cloned(),
                    "--since" => {
                        let raw = it.next().cloned().unwrap_or_default();
                        since = parse_since(&raw)?;
                    }
                    other if !other.starts_with("--") && !positional_taken => {
                        dir = other.to_string();
                        positional_taken = true;
                    }
                    other => return Err(format!("unexpected argument '{other}'\n{usage}")),
                }
            }
            if let Some(agent) = agent.as_deref() {
                if !matches!(agent, "claude" | "codex") {
                    return Err(format!("unknown agent '{agent}' (claude|codex)\n{usage}"));
                }
            }
            let home = home_dir().ok_or("cannot locate the home directory")?;
            let store = open_store()?;
            let out = sessions::import(
                &store,
                &home,
                std::path::Path::new(&dir),
                &prefix,
                agent.as_deref(),
                since,
            )
            .map_err(|e| e.to_string())?;
            println!(
                "sessions: {} imported, {} unchanged, {} ran in another workspace",
                out.imported, out.unchanged, out.elsewhere
            );
            Ok(())
        }
        Some("list") => {
            let prefix = args.get(1).ok_or(usage)?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let now = now_ms();
            let rows = sessions::list(&store, &index, prefix).map_err(|e| e.to_string())?;
            if rows.is_empty() {
                println!("no agent sessions under {prefix} (try: brain sessions import)");
                return Ok(());
            }
            for row in rows {
                let ago = now.saturating_sub(row.ended_at_ms) / 1000;
                let minutes = row.ended_at_ms.saturating_sub(row.started_at_ms) / 60_000;
                println!(
                    "[{ago:>6}s ago] {} ({}) {}min, {} turn(s), {} file(s): {}",
                    row.agent,
                    row.model.as_deref().unwrap_or("model unrecorded"),
                    minutes,
                    row.turns,
                    row.files_touched,
                    row.objective
                );
                if !row.tools.is_empty() {
                    println!("           tools: {}", row.tools);
                }
            }
            Ok(())
        }
        _ => Err(usage.to_string()),
    }
}

/// `30m`, `2h`, `7d`, or raw epoch milliseconds.
fn parse_since(raw: &str) -> Result<u64, String> {
    if raw.is_empty() {
        return Ok(0);
    }
    let now = now_ms();
    let (value, unit) = raw.split_at(raw.len() - 1);
    let scale = match unit {
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => {
            return raw
                .parse::<u64>()
                .map_err(|_| format!("cannot read '{raw}' as a time (try 30m, 2h, 7d, or epoch ms)"))
        }
    };
    let value: u64 = value
        .parse()
        .map_err(|_| format!("cannot read '{raw}' as a time (try 30m, 2h, 7d)"))?;
    Ok(now.saturating_sub(value * scale))
}

fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}

/// `brain testrun ...` — test protocols in the graph.
fn cmd_testrun(args: &[String]) -> Result<(), String> {
    use brain_observe::testing;
    let usage = "usage: brain testrun import <report-file|-> --prefix <p> [--dir <d>] | brain testrun list <prefix>";
    match args.first().map(String::as_str) {
        Some("import") => {
            let mut file = None;
            let mut prefix = None;
            let mut dir = ".".to_string();
            let mut it = args[1..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--prefix" => prefix = it.next().cloned(),
                    "--dir" => dir = it.next().cloned().unwrap_or_else(|| ".".to_string()),
                    other if file.is_none() => file = Some(other.to_string()),
                    other => return Err(format!("unexpected argument '{other}'\n{usage}")),
                }
            }
            let file = file.ok_or(usage)?;
            let prefix = prefix.ok_or_else(|| format!("--prefix is required\n{usage}"))?;
            let raw = if file == "-" {
                use std::io::Read as _;
                let mut buf = String::new();
                std::io::stdin()
                    .read_to_string(&mut buf)
                    .map_err(|e| e.to_string())?;
                buf
            } else {
                std::fs::read_to_string(&file).map_err(|e| format!("cannot read '{file}': {e}"))?
            };
            let report = testing::parse_report(&raw);
            if report.cases.is_empty() {
                return Err(
                    "no test cases recognized (expected cargo-test output, JUnit XML, or Playwright JSON)"
                        .to_string(),
                );
            }
            let store = open_store()?;
            let out = testing::record_run_in(
                &store,
                std::path::Path::new(&dir),
                &prefix,
                &report,
                &raw,
            )
            .map_err(|e| e.to_string())?;
            let state = if out.wrote {
                "recorded"
            } else {
                "already imported (unchanged)"
            };
            println!(
                "run {state}: {} total, {} passed, {} failed, {} skipped ({}); {} transition(s)",
                out.total, out.passed, out.failed, out.skipped, report.format, out.transitions
            );
            for name in &out.failing {
                println!("  FAILED {name}");
            }
            let attachments: usize = report.cases.iter().map(|c| c.attachments.len()).sum();
            if attachments > 0 {
                println!(
                    "  {attachments} attachment(s) linked to their cases — run `brain twin refresh {dir} --prefix {prefix}` to hash them"
                );
            }
            Ok(())
        }
        Some("list") => {
            let prefix = args.get(1).ok_or(usage)?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let now = now_ms();
            let runs = testing::runs(&store, &index, prefix).map_err(|e| e.to_string())?;
            if runs.is_empty() {
                println!("no test runs under {prefix}");
            }
            for (at, total, passed, failed, format) in runs {
                let age = now.saturating_sub(at) / 1000;
                let verdict = if failed == 0 { "ok" } else { "FAILED" };
                println!("[{age:>6}s ago] {verdict}: {passed}/{total} passed, {failed} failed ({format})");
            }
            Ok(())
        }
        _ => Err(usage.to_string()),
    }
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

fn describe(
    store: &Store,
    names: &std::collections::BTreeMap<NodeId, Vec<String>>,
    id: &NodeId,
) -> String {
    let kind = store.get(id).map(|o| kind_of(&o)).unwrap_or("missing");
    let bound = names
        .get(id)
        .map(|n| format!("  ({})", n.join(", ")))
        .unwrap_or_default();
    format!("{id:?}  {kind}{bound}")
}

/// The CLI's query backend: cortex — a persisted checkpoint plus
/// event-log delta replay, O(new events) on a warm open. It derefs to
/// MemIndex, so every query path below is written against the reference
/// backend. `BRAIN_INDEX=mem` forces a cold, non-persisting rebuild.
pub(crate) fn build_index(store: &Store) -> Result<cortex::Cortex, String> {
    if std::env::var("BRAIN_INDEX").as_deref() == Ok("mem") {
        return cortex::Cortex::open_ephemeral(store).map_err(|e| e.to_string());
    }
    let graf = cortex::Cortex::open(store).map_err(|e| e.to_string())?;
    // Best-effort persistence: a failed checkpoint costs only warmth.
    let _ = graf.checkpoint();
    // The object pack keeps the same bargain one level down: reading the
    // graph as one file instead of ten thousand. Also disposable, also
    // best-effort, also cheap once warm — only new objects are copied.
    let _ = store.compact();
    Ok(graf)
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
    let other_root = args.first().ok_or("usage: brain pull|push <store-root>")?;
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
        if let Object::Evidence {
            level,
            method,
            passed,
            detail,
            ..
        } = store.get(&id).map_err(|e| e.to_string())?
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
        other => {
            return Err(format!(
                "'{arg}' is not an entity (found {})",
                kind_of(&other)
            ))
        }
    };
    let index = build_index(&store)?;
    let mut rows = Vec::new();
    for obs_id in index.observations_of(&stable) {
        if let Object::Observation {
            property,
            value,
            source,
            observed_at_ms,
            ..
        } = store.get(&obs_id).map_err(|e| e.to_string())?
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
