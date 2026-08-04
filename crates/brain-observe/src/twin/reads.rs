//! The observation primitives every other part of the twin is built from: writing a
//! guarded fact, retracting an edge, and reading the latest — or the latest as of a moment.

use brain_core::ids::{NodeId, StableId};
use brain_core::object::Object;
use brain_index::{replay, Index, MemIndex};
use brain_store::{now_ms, Store, StoreError};
use std::collections::BTreeSet;

pub(crate) fn observe(
    store: &Store,
    subject: &StableId,
    property: &str,
    value: &str,
    at: u64,
) -> Result<NodeId, StoreError> {
    observe_src(store, subject, property, value, "twin", at)
}

pub(crate) fn observe_src(
    store: &Store,
    subject: &StableId,
    property: &str,
    value: &str,
    source: &str,
    at: u64,
) -> Result<NodeId, StoreError> {
    store.put(&Object::Observation {
        subject: subject.clone(),
        property: property.to_string(),
        value: value.to_string(),
        source: source.to_string(),
        observed_at_ms: at,
    })
}

/// Write a relation unless the graph (or this run) already has it live.
/// A relation whose edge was retracted is restored with an `active=true`
/// observation instead of a duplicate relation object.
pub(crate) fn relate(
    store: &Store,
    index: &MemIndex,
    written: &mut BTreeSet<(StableId, String, StableId)>,
    from: &StableId,
    kind: &str,
    to: &StableId,
    at: u64,
) -> Result<bool, StoreError> {
    let key = (from.clone(), kind.to_string(), to.clone());
    if written.contains(&key) {
        return Ok(false);
    }
    for id in index.relations_from(from, kind) {
        if let Object::Relation { to: t, .. } = store.get(&id)? {
            if &t == to {
                written.insert(key);
                if !brain_index::edge_active(index, store, from, kind, to)? {
                    observe(
                        store,
                        &brain_index::edge_sid(from, kind, to),
                        "active",
                        "true",
                        at,
                    )?;
                    return Ok(true);
                }
                return Ok(false);
            }
        }
    }
    store.put(&Object::Relation {
        from: from.clone(),
        predicate: kind.to_string(),
        to: to.clone(),
        source: "twin".to_string(),
        observed_at_ms: at,
    })?;
    written.insert(key);
    Ok(true)
}

/// Retract an edge: write `active=false` on its edge sid, guarded — a
/// no-op when the edge is already retracted or the relation never existed.
pub(crate) fn retract_edge(
    store: &Store,
    index: &MemIndex,
    from: &StableId,
    kind: &str,
    to: &StableId,
    source: &str,
    at: u64,
) -> Result<bool, StoreError> {
    let mut exists = false;
    for id in index.relations_from(from, kind) {
        if let Object::Relation { to: t, .. } = store.get(&id)? {
            if &t == to {
                exists = true;
                break;
            }
        }
    }
    if !exists || !brain_index::edge_active(index, store, from, kind, to)? {
        return Ok(false);
    }
    observe_src(
        store,
        &brain_index::edge_sid(from, kind, to),
        "active",
        "false",
        source,
        at,
    )?;
    Ok(true)
}

/// Agent-facing retraction: mark an edge as no longer holding (a wrong
/// `feature link`, a superseded association). The relation object stays —
/// history is never destroyed — but every reader stops seeing the edge.
pub fn retract(
    store: &Store,
    from: &StableId,
    kind: &str,
    to: &StableId,
) -> Result<bool, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    retract_edge(store, &index, from, kind, to, "agent", now_ms())
}

/// Live outgoing edges of one predicate: (relation node, target sid).
/// Retracted edges are skipped — this is how every reader sees only the
/// relations that still hold.
pub fn live_from(
    index: &MemIndex,
    store: &Store,
    from: &StableId,
    kind: &str,
) -> Result<Vec<(NodeId, StableId)>, StoreError> {
    live_from_at(index, store, from, kind, u64::MAX)
}

/// Outgoing edges of one predicate as they stood at `t`. An edge exists
/// at `t` only if it was recorded by then AND no retraction had landed
/// by then — with no retraction recorded an edge reads active at any
/// moment, so the recording time is the load-bearing half of the check.
pub fn live_from_at(
    index: &MemIndex,
    store: &Store,
    from: &StableId,
    kind: &str,
    t: u64,
) -> Result<Vec<(NodeId, StableId)>, StoreError> {
    let mut out = Vec::new();
    for id in index.relations_from(from, kind) {
        if let Object::Relation {
            to, observed_at_ms, ..
        } = store.get(&id)?
        {
            if observed_at_ms <= t
                && brain_index::edge_active_at(index, store, from, kind, &to, t)?
            {
                out.push((id, to));
            }
        }
    }
    Ok(out)
}

/// Live incoming edges of one predicate: (relation node, source sid).
pub fn live_to(
    index: &MemIndex,
    store: &Store,
    to: &StableId,
    kind: &str,
) -> Result<Vec<(NodeId, StableId)>, StoreError> {
    live_to_at(index, store, to, kind, u64::MAX)
}

/// Incoming edges of one predicate as they stood at `t`; the same
/// two-part check as `live_from_at`.
pub fn live_to_at(
    index: &MemIndex,
    store: &Store,
    to: &StableId,
    kind: &str,
    t: u64,
) -> Result<Vec<(NodeId, StableId)>, StoreError> {
    let mut out = Vec::new();
    for id in index.relations_to(to, kind) {
        if let Object::Relation {
            from,
            observed_at_ms,
            ..
        } = store.get(&id)?
        {
            if observed_at_ms <= t
                && brain_index::edge_active_at(index, store, &from, kind, to, t)?
            {
                out.push((id, from));
            }
        }
    }
    Ok(out)
}

/// Resolve a point in time: epoch ms, a relative `30m`/`2h`/`1d` ago, a
/// named baseline, a git commit hash looked up in the repo entity's
/// observation timeline, or the literal `live` — the present.
pub fn resolve_when(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    when: &str,
) -> Result<u64, String> {
    if when == "live" {
        return Ok(u64::MAX);
    }
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
    // A named baseline: the moment it recorded.
    if let Some(b) = crate::baseline::list(store, index, prefix)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|b| b.name == when)
    {
        return Ok(b.at_ms);
    }
    // A commit hash (prefix): when the twin observed that commit as HEAD.
    let repo = StableId::derive(&["repo", prefix]);
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
        "cannot resolve '{when}' (epoch ms, 30m/2h/1d, a baseline name, or a twinned commit hash)"
    ))
}

/// Retract live edges of `kinds` from `from` that this pass did not
/// re-observe (their key is absent from `written`). Guarded: an edge
/// already retracted writes nothing.
pub(crate) fn sweep_edges(
    store: &Store,
    index: &MemIndex,
    written: &BTreeSet<(StableId, String, StableId)>,
    from: &StableId,
    kinds: &[&str],
    at: u64,
) -> Result<usize, StoreError> {
    let mut retracted = 0;
    for kind in kinds {
        for (_, to) in live_from(index, store, from, kind)? {
            if !written.contains(&(from.clone(), kind.to_string(), to.clone()))
                && retract_edge(store, index, from, kind, &to, "twin", at)?
            {
                retracted += 1;
            }
        }
    }
    Ok(retracted)
}

/// A human-readable label for an entity: path, name, or slug — else the id.
pub fn sid_label(index: &MemIndex, store: &Store, sid: &StableId) -> String {
    for node in index.entity_nodes(sid) {
        if let Ok(Object::Entity { labels, .. }) = store.get(&node) {
            for key in ["path", "name", "slug"] {
                if let Some(v) = labels.get(key) {
                    return v.clone();
                }
            }
        }
    }
    sid.to_string()
}

/// Latest observation value for (subject, property), by observation time.
pub fn latest(
    index: &MemIndex,
    store: &Store,
    subject: &StableId,
    property: &str,
) -> Result<Option<String>, StoreError> {
    Ok(latest_at(index, store, subject, property)?.map(|(_, v)| v))
}

/// Like [`latest`], but also returns when the value was observed.
pub fn latest_at(
    index: &MemIndex,
    store: &Store,
    subject: &StableId,
    property: &str,
) -> Result<Option<(u64, String)>, StoreError> {
    latest_at_before(index, store, subject, property, u64::MAX)
}

/// Like [`latest_at`], but also returns who wrote the value — the seed
/// upgrade rule needs to distinguish shipped defaults from local edits.
pub fn latest_with_source(
    index: &MemIndex,
    store: &Store,
    subject: &StableId,
    property: &str,
) -> Result<Option<(u64, String, String)>, StoreError> {
    let mut best: Option<(u64, String, String)> = None;
    for id in index.observations_of(subject) {
        if let Object::Observation {
            property: p,
            value,
            source,
            observed_at_ms,
            ..
        } = store.get(&id)?
        {
            if p == property && best.as_ref().is_none_or(|(b, _, _)| observed_at_ms >= *b) {
                best = Some((observed_at_ms, value, source));
            }
        }
    }
    Ok(best)
}

/// Bi-temporal read: the newest observation at or before `t`. Every
/// observation carries its time, so "as of" is a filter, not a replay.
pub fn latest_at_before(
    index: &MemIndex,
    store: &Store,
    subject: &StableId,
    property: &str,
    t: u64,
) -> Result<Option<(u64, String)>, StoreError> {
    let mut best: Option<(u64, String)> = None;
    for id in index.observations_of(subject) {
        if let Object::Observation {
            property: p,
            value,
            observed_at_ms,
            ..
        } = store.get(&id)?
        {
            if p == property
                && observed_at_ms <= t
                && best.as_ref().is_none_or(|(b, _)| observed_at_ms >= *b)
            {
                best = Some((observed_at_ms, value));
            }
        }
    }
    Ok(best)
}

/// The twin as it was: files present under `prefix` as of `t`, with the
/// content hash each had at that moment.
pub fn files_at(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    t: u64,
) -> Result<Vec<(String, String)>, StoreError> {
    let mut out = Vec::new();
    for (name, node) in store.namespace()? {
        let Some(rel) = name.strip_prefix(&format!("{prefix}/")) else {
            continue;
        };
        let Ok(Object::Entity {
            id: sid,
            entity_kind,
            ..
        }) = store.get(&node)
        else {
            continue;
        };
        if entity_kind != "source_file" {
            continue;
        }
        let Some((_, hash)) = latest_at_before(index, store, &sid, "content_b3", t)? else {
            continue; // did not exist yet at t
        };
        if latest_at_before(index, store, &sid, "present", t)?
            .map(|(_, v)| v)
            .as_deref()
            == Some("false")
        {
            continue; // was deleted at t
        }
        out.push((rel.to_string(), hash));
    }
    Ok(out)
}
