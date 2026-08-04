//! Shared plumbing every command group leans on: opening the store, resolving
//! arguments, and the small parsers for common flags.

use brain_core::ids::NodeId;
use brain_core::object::Object;
use brain_index::{Index, MemIndex};
use brain_store::Store;

pub(crate) fn open_store() -> Result<Store, String> {
    let root = std::env::var("BRAIN_STORE").unwrap_or_else(|_| ".brain".to_string());
    Store::open(root).map_err(|e| e.to_string())
}

/// Open an existing store at an explicit path (for sync). Refuses to
/// conjure an empty store out of a typo.
pub(crate) fn open_existing_store(root: &str) -> Result<Store, String> {
    if !std::path::Path::new(root).join("objects").is_dir() {
        return Err(format!("no store at '{root}' (missing objects/)"));
    }
    Store::open(root).map_err(|e| e.to_string())
}

pub(crate) fn parse_prefix(args: &[String]) -> String {
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

/// Resolve a bound name to the entity's stable id.
pub(crate) fn entity_sid(store: &Store, name: &str) -> Result<brain_core::ids::StableId, String> {
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
pub(crate) fn relation_targets(
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
pub(crate) fn resolve_entity(
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
pub(crate) fn positional(args: &[String]) -> Vec<&String> {
    let mut out = Vec::new();
    let mut skip = false;
    for arg in args {
        if skip {
            skip = false;
            continue;
        }
        if let Some(flag) = arg.strip_prefix("--") {
            // Only value-taking flags swallow the next argument.
            skip = matches!(
                flag,
                "prefix" | "why" | "note" | "title" | "kind" | "file" | "top" | "objective"
                    | "outcome"
            );
            continue;
        }
        out.push(arg);
    }
    out
}

/// A human-readable label for an entity: its path, name, or raw stable id.
pub(crate) fn entity_label(store: &Store, index: &MemIndex, sid: &brain_core::ids::StableId) -> String {
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

/// Resolve a point in time — the shared resolver in the observe layer:
/// epoch ms, relative `30m`/`2h`/`1d`, a baseline name, or a commit.
pub(crate) fn resolve_when(store: &Store, index: &MemIndex, prefix: &str, when: &str) -> Result<u64, String> {
    brain_observe::twin::resolve_when(store, index, prefix, when)
}

pub(crate) fn parse_top(args: &[String], default: usize) -> Result<usize, String> {
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

/// `--json`: the same answer as data. Queries render prose for people and
/// serialize the identical structure for agents — one projection each,
/// never a second source of truth.
pub(crate) fn wants_json(args: &[String]) -> bool {
    args.iter().any(|a| a == "--json")
}

/// The prefix is the longest bound repo entity whose name prefixes ours
/// (twin/self/src/main.rs -> twin/self).
pub(crate) fn twin_prefix_of(store: &Store, name: &str) -> Result<String, String> {
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
    Ok(prefix)
}

/// Resolve a CLI argument that may be a bound name or a literal b3: hash.
pub(crate) fn resolve_arg(store: &Store, arg: &str) -> Result<NodeId, String> {
    if arg.starts_with("b3:") {
        return NodeId::parse(arg).map_err(|e| e.to_string());
    }
    store
        .resolve(arg)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no binding for '{arg}'"))
}

/// Names currently bound to each node, for human-readable listings.
pub(crate) fn names_of(store: &Store) -> Result<std::collections::BTreeMap<NodeId, Vec<String>>, String> {
    let mut out: std::collections::BTreeMap<NodeId, Vec<String>> = Default::default();
    for (name, id) in store.namespace().map_err(|e| e.to_string())? {
        out.entry(id).or_default().push(name);
    }
    Ok(out)
}

pub(crate) fn kind_of(obj: &Object) -> &'static str {
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

pub(crate) fn describe(
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
pub(crate) fn build_index(store: &Store) -> Result<brain_cortex::Cortex, String> {
    if std::env::var("BRAIN_INDEX").as_deref() == Ok("mem") {
        return brain_cortex::Cortex::open_ephemeral(store).map_err(|e| e.to_string());
    }
    let graf = brain_cortex::Cortex::open(store).map_err(|e| e.to_string())?;
    // Best-effort persistence: a failed checkpoint costs only warmth.
    let _ = graf.checkpoint();
    // The object pack keeps the same bargain one level down: reading the
    // graph as one file instead of ten thousand. Also disposable, also
    // best-effort, also cheap once warm — only new objects are copied.
    let _ = store.compact();
    Ok(graf)
}
