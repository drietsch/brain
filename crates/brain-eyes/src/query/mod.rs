//! Read-only projections of the graph.
//!
//! Every query here goes through the held `Cortex` index and the existing
//! `brain_observe` functions. Nothing re-implements a judgment that the
//! workspace already computes — that is how Eyes and the CLI stay in
//! agreement, and how a dossier stopped costing nine passes over the store.

pub mod evidence;
pub mod features;
pub mod find;
pub mod library;
pub mod map;
pub mod media;
pub mod mri;
pub mod now;
pub mod tests;
pub mod thing;
pub mod timeline;
pub mod work;

use crate::dto::Ref;
use crate::say;
use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::{Index, MemIndex};
use brain_observe::twin;
use brain_store::Store;
use std::collections::{BTreeMap, BTreeSet};

/// The one prefix rule. `twin/self` must not match `twin/selfhosted`.
pub fn in_prefix(name: &str, prefix: &str) -> bool {
    name == prefix || name.starts_with(&format!("{prefix}/"))
}

/// Entities of one kind scoped to a prefix, deduplicated, index-backed.
pub fn scoped(
    index: &MemIndex,
    store: &Store,
    prefix: &str,
    kind: &str,
) -> Result<Vec<(StableId, BTreeMap<String, String>)>, String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for node in index.entities_by_kind(kind) {
        let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
            continue;
        };
        if labels.get("prefix").map(String::as_str) != Some(prefix) || !seen.insert(id.clone()) {
            continue;
        }
        out.push((id, labels));
    }
    Ok(out)
}

/// Twinned files under the prefix that are still present, as
/// (relative path, stable id).
pub fn present_files(
    index: &MemIndex,
    store: &Store,
    prefix: &str,
) -> Result<Vec<(String, StableId)>, String> {
    let mut out = Vec::new();
    for (name, node) in store.namespace().map_err(|e| e.to_string())? {
        if !in_prefix(&name, prefix) {
            continue;
        }
        let Some(rel) = name.strip_prefix(&format!("{prefix}/")) else {
            continue;
        };
        let Ok(Object::Entity {
            id, entity_kind, ..
        }) = store.get(&node)
        else {
            continue;
        };
        if entity_kind != "source_file" {
            continue;
        }
        if twin::latest(index, store, &id, "present")
            .map_err(|e| e.to_string())?
            .as_deref()
            == Some("false")
        {
            continue;
        }
        out.push((rel.to_string(), id));
    }
    Ok(out)
}

/// The entity kind of a stable id, when the graph knows one.
pub fn kind_of(index: &MemIndex, store: &Store, sid: &StableId) -> Option<String> {
    for node in index.entity_nodes(sid) {
        if let Ok(Object::Entity { entity_kind, .. }) = store.get(&node) {
            return Some(entity_kind);
        }
    }
    None
}

/// Labels of an entity, merged newest-last.
pub fn labels_of(
    index: &MemIndex,
    store: &Store,
    sid: &StableId,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for node in index.entity_nodes(sid) {
        if let Ok(Object::Entity { labels, .. }) = store.get(&node) {
            out.extend(labels);
        }
    }
    out
}

/// A pointer the browser can open, with its human noun and glyph.
pub fn make_ref(index: &MemIndex, store: &Store, sid: &StableId) -> Ref {
    let kind = kind_of(index, store, sid).unwrap_or_default();
    let noun = if kind.is_empty() {
        "entity".to_string()
    } else {
        say::kind_noun(&kind).to_string()
    };
    // `sid_label` falls back to the raw identifier when an entity carries
    // no display label — a repository, for instance. Never show that.
    let mut label = twin::sid_label(index, store, sid);
    if label.starts_with("sid:") {
        let labels = labels_of(index, store, sid);
        // An agent session carries no name of its own, and the prefix is
        // the same for every one of them — so a session labelled by its
        // prefix reads as the repository. Say who ran, and which run.
        if kind == "agent_session" {
            let agent = labels.get("agent").cloned().unwrap_or_default();
            let run: String = labels
                .get("session_id")
                .map(|id| id.chars().take(6).collect())
                .unwrap_or_default();
            if !agent.is_empty() {
                label = if run.is_empty() {
                    agent
                } else {
                    format!("{agent} {run}")
                };
                return Ref {
                    id: sid.to_string(),
                    label,
                    kind: kind.clone(),
                    noun,
                    glyph: say::kind_glyph(&kind).to_string(),
                };
            }
        }
        // `prefix` comes last: it identifies the workspace, not the thing.
        label = ["title", "name", "slug", "path", "prefix"]
            .iter()
            .find_map(|key| labels.get(*key).cloned())
            .unwrap_or_else(|| noun.clone());
    }
    Ref {
        id: sid.to_string(),
        label,
        kind: kind.clone(),
        noun,
        glyph: say::kind_glyph(&kind).to_string(),
    }
}

/// A display title: the recorded title, else the label.
pub fn title_of(
    index: &MemIndex,
    store: &Store,
    sid: &StableId,
    labels: &BTreeMap<String, String>,
) -> String {
    if let Ok(Some(title)) = twin::latest(index, store, sid, "title") {
        if !title.trim().is_empty() {
            return title;
        }
    }
    if let Some(title) = labels.get("title") {
        return title.clone();
    }
    twin::sid_label(index, store, sid)
}

/// What to call this thing in a sentence. Usually its title — but a
/// one-word title ("# brain" at the top of a README) identifies nothing,
/// so fall back to the filename a developer would actually go and open.
pub fn display_name(
    index: &MemIndex,
    store: &Store,
    sid: &StableId,
    labels: &BTreeMap<String, String>,
) -> String {
    let title = title_of(index, store, sid, labels);
    if title.split_whitespace().count() <= 1 {
        if let Some(path) = labels.get("path") {
            return path.rsplit('/').next().unwrap_or(path).to_string();
        }
    }
    title
}

/// The newest `content` observation time for an entity, used to order
/// reading lists by recency.
pub fn changed_at(index: &MemIndex, store: &Store, sid: &StableId) -> u64 {
    for property in ["content", "content_b3", "status", "result"] {
        if let Ok(Some((at, _))) = twin::latest_at(index, store, sid, property) {
            return at;
        }
    }
    0
}

/// First meaningful lines of a body, for shelves you read rather than scan.
pub fn excerpt(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with('#')
            || line.starts_with("---")
            || line.starts_with("<!--")
            || line.starts_with("Status:")
            || line.starts_with("Service:")
        {
            continue;
        }
        out.push_str(line);
        if out.chars().count() >= max_chars {
            break;
        }
        out.push(' ');
    }
    let trimmed: String = out.trim().chars().take(max_chars).collect();
    if trimmed.chars().count() == max_chars {
        format!("{trimmed}…")
    } else {
        trimmed
    }
}

/// The features this thing serves, as pointers a row can open.
///
/// Empty is the ordinary answer and renders as nothing: most of a graph
/// is not claimed by anything, and saying so on every row would be noise.
pub fn features_of(loaded: &crate::state::Loaded, sid: &StableId) -> Vec<Ref> {
    loaded
        .spine()
        .features_of(sid)
        .iter()
        .map(|owned| make_ref(&loaded.index, &loaded.store, &owned.feature))
        .collect()
}
