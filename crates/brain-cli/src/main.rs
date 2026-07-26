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
       brain twin insights <prefix>              synthesized picture: churn, hubs, growth\n\
       brain note <name> <text...>        attach a durable note to an entity\n\
       brain notes <name>                 read an entity's notes\n\
       brain adr add <md-file> --prefix <p> [--title T] [--status S]   record a decision\n\
       brain plan add <md-file> --prefix <p> [--title T]               record a plan\n\
       brain adr|plan list <prefix>       decisions/plans under a prefix\n\
       brain adr|plan show <prefix> <slug>   full document, status timeline, mentions\n\
       brain skill add <SKILL.md> --prefix <p>       record an agent skill\n\
       brain agentcfg add <file> --prefix <p> [--agent A] [--role R]   record agent config\n\
       brain skill|agentcfg list <prefix> | show <prefix> <slug>       browse them\n\
       brain template seed|list|show <slug>          the deliverable contract, in the graph\n\
       brain deliverable new <template> [--title T]  instantiate a scaffold to stdout\n\
       brain feature add <prefix> <slug> [--title T] [--status S]      register a feature\n\
       brain feature link <prefix> <slug> <predicate> <target>         link into the graph\n\
       brain feature list|matrix <prefix>            registry / rendered DoD matrix\n\
       brain done <prefix> <slug>                    evaluate a feature against the DoD\n\
       brain testrun import <report> --prefix <p>    ingest cargo-test output or JUnit XML\n\
       brain testrun list <prefix>                   imported protocols, newest first\n\
       brain twin tests <prefix>                     test files, frameworks, failing cases\n\
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
        Some("adr") => cmd_doc(&args[1..], brain_observe::docs::DocKind::Decision),
        Some("plan") => cmd_doc(&args[1..], brain_observe::docs::DocKind::Plan),
        Some("skill") => cmd_agent_doc(&args[1..], brain_observe::agents::AgentDocKind::Skill),
        Some("agentcfg") => cmd_agent_doc(&args[1..], brain_observe::agents::AgentDocKind::Config),
        Some("template") => cmd_template(&args[1..]),
        Some("deliverable") => cmd_deliverable(&args[1..]),
        Some("feature") => cmd_feature(&args[1..]),
        Some("done") => cmd_done(&args[1..]),
        Some("testrun") => cmd_testrun(&args[1..]),
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
    for f in &report.docs {
        println!("doc      {f}");
    }
    let verb = if wrote { "recorded" } else { "would record" };
    println!(
        "{} unchanged; {verb} {} added, {} changed, {} deleted ({} symbols, {} relations, {} docs)",
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
        Some("insights") => {
            let prefix = args.get(1).ok_or("usage: brain twin insights <prefix>")?;
            let store = open_store()?;
            let ins = brain_observe::twin::insights(&store, prefix).map_err(|e| e.to_string())?;
            let now = now_ms();
            println!("== twin insights: {prefix} ==");
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
            if !ins.failing.is_empty() {
                println!("failing tests:");
                for name in &ins.failing {
                    println!("  ✗ {name}");
                }
            }
            let list = |title: &str, items: &[(String, usize)], unit: &str| {
                if !items.is_empty() {
                    println!("{title}:");
                    for (name, n) in items {
                        println!("  {n:>4} {unit}  {name}");
                    }
                }
            };
            if !ins.churn.is_empty() {
                println!("churn (most edited):");
                for (name, n) in &ins.churn {
                    let tag = if ins.decided.contains(name) { "  [decided]" } else { "" };
                    println!("  {n:>4} versions  {name}{tag}");
                }
            }
            list("hubs (most imported)", &ins.hubs, "importers");
            list("untested hubs (imported, no tests)", &ins.untested_hubs, "importers");
            list("largest (symbols declared)", &ins.largest, "symbols");
            list("external deps (unresolved imports)", &ins.external_modules, "uses");
            if !ins.decisions.is_empty() {
                println!("decisions (ADRs):");
                for (slug, title, status) in &ins.decisions {
                    println!("  [{status}] {slug}: {title}");
                }
            }
            if !ins.plans.is_empty() {
                println!("plans:");
                for (slug, title) in &ins.plans {
                    println!("  {slug}: {title}");
                }
            }
            if !ins.skills.is_empty() {
                println!("agent skills:");
                for (slug, agent, desc) in &ins.skills {
                    println!("  [{agent}] {slug}: {desc}");
                }
            }
            if !ins.agent_configs.is_empty() {
                println!("agent config:");
                for (slug, agent, role) in &ins.agent_configs {
                    println!("  [{agent}] {slug} ({role})");
                }
            }
            if !ins.features.is_empty() {
                println!("features (DoD progress):");
                for (slug, status, fraction) in &ins.features {
                    println!("  [{status}] {slug}  {fraction}");
                }
            }
            if !ins.nonconforming.is_empty() {
                println!("nonconforming docs (template contract):");
                for (slug, kind, missing) in &ins.nonconforming {
                    println!("  {slug} ({kind}): missing {missing}");
                }
            }
            if !ins.notes.is_empty() {
                println!("recent notes:");
                for (at, entity, text) in &ins.notes {
                    let age = now.saturating_sub(*at) / 1000;
                    println!("  [{age}s ago] {entity}: {text}");
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
                let Some(rel) = name.strip_prefix(&format!("{prefix}/")) else { continue };
                let Ok(Object::Entity { id: sid, .. }) = store.get(&node) else { continue };
                let Some(framework) = brain_observe::twin::latest(&index, &store, &sid, "test_framework")
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
         brain {cmd} list <prefix> | brain {cmd} show <prefix> <slug>",
        if cmd == "adr" { " [--status S]" } else { "" }
    );
    match args.first().map(String::as_str) {
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
            let content = std::fs::read_to_string(&file)
                .map_err(|e| format!("cannot read '{file}': {e}"))?;
            let slug = docs::slug_of(&file);
            let meta =
                docs::parse_content(kind, &slug, &content, title.as_deref(), status.as_deref());
            let store = open_store()?;
            let out = twin::add_doc(&store, &prefix, &meta, &content, "claude-code")
                .map_err(|e| e.to_string())?;
            let state = if out.wrote { "recorded" } else { "already recorded (unchanged)" };
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
            let prefix = args.get(1).ok_or_else(|| usage.clone())?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let now = now_ms();
            let mut seen = BTreeSet::new();
            let mut any = false;
            for node in index.entities_by_kind(noun) {
                let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else { continue };
                if labels.get("prefix") != Some(prefix) || !seen.insert(id.clone()) {
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
                println!("{status}{slug}: {title}  ({age}, {mentions} mention(s))");
                any = true;
            }
            if !any {
                println!("no {noun}s under {prefix}");
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
            // Status timeline (decisions), oldest first.
            let mut statuses = Vec::new();
            for id in index.observations_of(&sid) {
                if let Object::Observation { property, value, observed_at_ms, .. } =
                    store.get(&id).map_err(|e| e.to_string())?
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
            let content = std::fs::read_to_string(&file)
                .map_err(|e| format!("cannot read '{file}': {e}"))?;
            // Detect slug/agent/role from the path conventions when they
            // apply, then let explicit flags override.
            let normalized = file.replace('\\', "/");
            let (slug, det_agent, det_role) = match agents::path_agent_doc(&normalized) {
                Some((k, slug, a, r)) if k == kind => (slug, a, r),
                _ => {
                    let stem = normalized.rsplit('/').next().unwrap_or(&normalized);
                    let stem = stem.strip_suffix(".md").unwrap_or(stem).to_lowercase();
                    let default_role =
                        if cmd == "skill" { "skill" } else { "instructions" };
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
            let state = if out.wrote { "recorded" } else { "already recorded (unchanged)" };
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
            let prefix = args.get(1).ok_or_else(|| usage.clone())?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let mut seen = BTreeSet::new();
            let mut any = false;
            for node in index.entities_by_kind(noun) {
                let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else { continue };
                if labels.get("prefix") != Some(prefix) || !seen.insert(id.clone()) {
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
                println!("[{agent}] {slug} ({role}){desc}");
                any = true;
            }
            if !any {
                println!("no {noun}s under {prefix}");
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
    let usage = "usage: brain template seed | list | show <slug>";
    match args.first().map(String::as_str) {
        Some("seed") => {
            let store = open_store()?;
            let n = templates::seed(&store).map_err(|e| e.to_string())?;
            println!(
                "{} templates present; {n} observation(s) written",
                templates::DEFAULTS.len()
            );
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
                println!("{applies:<12} requires [{}]  — {title}", requires.join(", "));
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

/// `brain feature ...` — the registry: features as entities, links as edges.
fn cmd_feature(args: &[String]) -> Result<(), String> {
    use brain_observe::features;
    let usage = "usage: brain feature add <prefix> <slug> [--title T] [--status S] | \
                 feature link <prefix> <slug> <predicate> <target> | \
                 feature list <prefix> | feature matrix <prefix>";
    match args.first().map(String::as_str) {
        Some("add") => {
            let (prefix, slug) = match (args.get(1), args.get(2)) {
                (Some(p), Some(s)) => (p, s),
                _ => return Err(usage.to_string()),
            };
            let mut title = slug.clone();
            let mut status = "planned".to_string();
            let mut it = args[3..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--title" => title = it.next().cloned().unwrap_or(title),
                    "--status" => status = it.next().cloned().unwrap_or(status),
                    other => return Err(format!("unexpected argument '{other}'\n{usage}")),
                }
            }
            let store = open_store()?;
            let (_, wrote) = features::add(&store, prefix, slug, &title, &status)
                .map_err(|e| e.to_string())?;
            let state = if wrote { "recorded" } else { "unchanged" };
            println!("feature '{slug}' {state} under {prefix} (status: {status})");
            Ok(())
        }
        Some("link") => {
            let (prefix, slug, predicate, target) =
                match (args.get(1), args.get(2), args.get(3), args.get(4)) {
                    (Some(p), Some(s), Some(pr), Some(t)) => (p, s, pr, t),
                    _ => return Err(usage.to_string()),
                };
            let store = open_store()?;
            let index = build_index(&store)?;
            let (target_sid, kind) = features::resolve_target(&index, prefix, target)
                .ok_or_else(|| {
                    format!("no twinned entity matches '{target}' (file path, or a decision/plan/skill/feature slug)")
                })?;
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
                let met = report.checks.iter().filter(|c| c.count > 0).count();
                let done = if report.done { " ✓ done" } else { "" };
                println!(
                    "[{}] {}: {}  ({met}/{}{done})",
                    row.status,
                    row.slug,
                    row.title,
                    report.checks.len()
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
    for check in &report.checks {
        let mark = if check.count > 0 { "✓" } else { "✗" };
        println!("{mark} {}  ({} link(s))", check.predicate, check.count);
    }
    println!("{}: {}", slug, if report.done { "DONE" } else { "not done" });
    features::record_done(&store, &index, prefix, slug, &report).map_err(|e| e.to_string())?;
    Ok(())
}

/// `brain testrun ...` — test protocols in the graph.
fn cmd_testrun(args: &[String]) -> Result<(), String> {
    use brain_observe::testing;
    let usage = "usage: brain testrun import <report-file|-> --prefix <p> | brain testrun list <prefix>";
    match args.first().map(String::as_str) {
        Some("import") => {
            let mut file = None;
            let mut prefix = None;
            let mut it = args[1..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--prefix" => prefix = it.next().cloned(),
                    other if file.is_none() => file = Some(other.to_string()),
                    other => return Err(format!("unexpected argument '{other}'\n{usage}")),
                }
            }
            let file = file.ok_or(usage)?;
            let prefix = prefix.ok_or_else(|| format!("--prefix is required\n{usage}"))?;
            let raw = if file == "-" {
                use std::io::Read as _;
                let mut buf = String::new();
                std::io::stdin().read_to_string(&mut buf).map_err(|e| e.to_string())?;
                buf
            } else {
                std::fs::read_to_string(&file).map_err(|e| format!("cannot read '{file}': {e}"))?
            };
            let report = testing::parse_report(&raw);
            if report.cases.is_empty() {
                return Err("no test cases recognized (expected cargo-test output or JUnit XML)"
                    .to_string());
            }
            let store = open_store()?;
            let out =
                testing::record_run(&store, &prefix, &report, &raw).map_err(|e| e.to_string())?;
            let state = if out.wrote { "recorded" } else { "already imported (unchanged)" };
            println!(
                "run {state}: {} total, {} passed, {} failed, {} skipped ({}); {} transition(s)",
                out.total, out.passed, out.failed, out.skipped, report.format, out.transitions
            );
            for name in &out.failing {
                println!("  FAILED {name}");
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
