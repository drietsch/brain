//! Deliverables recorded as entities: decisions, plans, agent configuration, artifacts.

use super::*;
use crate::agents::AgentDoc;
use crate::docs::DocMeta;
use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::{replay, MemIndex};
use brain_store::{now_ms, Store, StoreError};
use std::collections::{BTreeMap, BTreeSet};

/// Stable identity of a decision/plan under a prefix. Derived from kind +
/// prefix + slug, not from the file path: the document's identity survives
/// file moves, and out-of-repo documents get the same scheme.
pub fn doc_sid(prefix: &str, meta: &DocMeta) -> StableId {
    StableId::derive(&[meta.kind.as_str(), prefix, &meta.slug])
}

#[derive(Debug)]
pub struct DocOutcome {
    pub sid: StableId,
    /// Did this call write anything (observations or relations)?
    pub wrote: bool,
    /// Relations written by this call.
    pub relations: usize,
    /// Edges retracted by this call (mentions dropped from the text,
    /// recorded_in of a former location).
    pub retracted: usize,
    /// Twinned file paths this document mentions.
    pub mentions: Vec<String>,
}

/// Stable identity of a skill/agent-config document under a prefix.
pub fn agent_doc_sid(prefix: &str, doc: &AgentDoc) -> StableId {
    StableId::derive(&[doc.kind.as_str(), prefix, &doc.slug])
}

/// The shared recording core for semantic documents (decisions, plans,
/// skills, agent configuration). Every observation is guarded by [`latest`],
/// so re-recording an unchanged document writes nothing — and a changed
/// property value becomes a new observation: a timeline, never an overwrite.
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_entity_doc(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    entity_kind: &str,
    slug: &str,
    extra_labels: &[(&str, &str)],
    props: &[(&str, &str)],
    content: &str,
    source: &str,
    rel_path: Option<&str>,
    candidates: &BTreeSet<String>,
    written_relations: &mut BTreeSet<(StableId, String, StableId)>,
    now: u64,
) -> Result<DocOutcome, StoreError> {
    let sid = StableId::derive(&[entity_kind, prefix, slug]);
    let mut labels = BTreeMap::new();
    labels.insert("prefix".to_string(), prefix.to_string());
    labels.insert("slug".to_string(), slug.to_string());
    for (k, v) in extra_labels {
        labels.insert(k.to_string(), v.to_string());
    }
    if let Some(rel) = rel_path {
        labels.insert("path".to_string(), rel.to_string());
    }
    store.put(&Object::Entity {
        id: sid.clone(),
        entity_kind: entity_kind.to_string(),
        labels,
    })?;

    let mut wrote = false;
    for (prop, value) in props {
        if latest(index, store, &sid, prop)?.as_deref() != Some(*value) {
            observe_src(store, &sid, prop, value, source, now)?;
            wrote = true;
        }
    }

    let mut outcome = DocOutcome {
        sid: sid.clone(),
        wrote,
        relations: 0,
        retracted: 0,
        mentions: Vec::new(),
    };

    let repo_sid = StableId::derive(&["repo", prefix]);
    if relate(
        store,
        index,
        written_relations,
        &sid,
        "concerns",
        &repo_sid,
        now,
    )? {
        outcome.relations += 1;
    }
    // Auto-detected documents keep their file entity too: the document is
    // the semantic thing, the file is where it happens to be recorded.
    if let Some(rel) = rel_path {
        let file_sid = StableId::derive(&["file", rel]);
        if relate(
            store,
            index,
            written_relations,
            &sid,
            "recorded_in",
            &file_sid,
            now,
        )? {
            outcome.relations += 1;
        }
    }
    // Mentions-scan: link the document to every twinned file its text names.
    for cand in candidates {
        if Some(cand.as_str()) == rel_path || !content.contains(cand.as_str()) {
            continue;
        }
        let file_sid = StableId::derive(&["file", cand]);
        if relate(
            store,
            index,
            written_relations,
            &sid,
            "mentions",
            &file_sid,
            now,
        )? {
            outcome.relations += 1;
        }
        outcome.mentions.push(cand.clone());
    }

    // Conformance against the graph-defined template for this kind, when
    // one is seeded: recorded as observations, never enforcement.
    if let Some((tmpl_sid, requires)) = crate::templates::by_kind(store, index)?.get(entity_kind) {
        let missing = crate::templates::check(content, requires);
        let conforms = if missing.is_empty() { "true" } else { "false" };
        if latest(index, store, &sid, "conforms")?.as_deref() != Some(conforms) {
            observe_src(store, &sid, "conforms", conforms, source, now)?;
            outcome.wrote = true;
        }
        let missing_val = missing.join(",");
        let prior = latest(index, store, &sid, "missing")?;
        if prior.as_deref() != Some(missing_val.as_str())
            && (!missing_val.is_empty() || prior.is_some())
        {
            observe_src(store, &sid, "missing", &missing_val, source, now)?;
            outcome.wrote = true;
        }
        if relate(
            store,
            index,
            written_relations,
            &sid,
            "conforms_to",
            tmpl_sid,
            now,
        )? {
            outcome.relations += 1;
        }
        // Version-precise conformance: record WHICH contract judged this
        // artifact, so template fitness can compare versions later.
        if let Some(contract) = latest(index, store, tmpl_sid, "contract_b3")? {
            if latest(index, store, &sid, "template_b3")?.as_deref() != Some(contract.as_str()) {
                observe_src(store, &sid, "template_b3", &contract, source, now)?;
                outcome.wrote = true;
            }
        }
    }

    // Currency sweep: a mention whose path the text no longer names is
    // retracted; a mention of a deleted file the text still names stays
    // live (that mismatch is a coherence finding, not stale structure).
    for (_, to) in live_from(index, store, &sid, "mentions")? {
        let path = sid_label(index, store, &to);
        if !content.contains(&path)
            && retract_edge(store, index, &sid, "mentions", &to, source, now)?
        {
            outcome.retracted += 1;
        }
    }
    // A moved document re-attaches to its new location; the old one is
    // retracted (doc identity is kind+prefix+slug, not the path).
    if let Some(rel) = rel_path {
        let here = StableId::derive(&["file", rel]);
        for (_, to) in live_from(index, store, &sid, "recorded_in")? {
            if to != here && retract_edge(store, index, &sid, "recorded_in", &to, source, now)? {
                outcome.retracted += 1;
            }
        }
    }

    if outcome.relations > 0 || outcome.retracted > 0 {
        outcome.wrote = true;
    }
    Ok(outcome)
}

/// Record a decision/plan document into the twin. A changed `Status:` line
/// becomes a new `status` observation: decisions get a timeline for free.
#[allow(clippy::too_many_arguments)]
pub fn record_doc(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    meta: &DocMeta,
    content: &str,
    source: &str,
    rel_path: Option<&str>,
    candidates: &BTreeSet<String>,
    written_relations: &mut BTreeSet<(StableId, String, StableId)>,
    now: u64,
) -> Result<DocOutcome, StoreError> {
    let mut props: Vec<(&str, &str)> = vec![("content", content), ("title", &meta.title)];
    if let Some(status) = &meta.status {
        props.push(("status", status));
    }
    let mut outcome = record_entity_doc(
        store,
        index,
        prefix,
        meta.kind.as_str(),
        &meta.slug,
        &[("title", &meta.title)],
        &props,
        content,
        source,
        rel_path,
        candidates,
        written_relations,
        now,
    )?;
    if let Some(other) = &meta.supersedes {
        let other_sid = StableId::derive(&["decision", prefix, other]);
        if relate(
            store,
            index,
            written_relations,
            &outcome.sid,
            "supersedes",
            &other_sid,
            now,
        )? {
            outcome.relations += 1;
            outcome.wrote = true;
        }
    }
    Ok(outcome)
}

/// Record a skill or agent-configuration document into the twin.
#[allow(clippy::too_many_arguments)]
pub fn record_agent_doc(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    doc: &AgentDoc,
    content: &str,
    source: &str,
    rel_path: Option<&str>,
    candidates: &BTreeSet<String>,
    written_relations: &mut BTreeSet<(StableId, String, StableId)>,
    now: u64,
) -> Result<DocOutcome, StoreError> {
    let mut props: Vec<(&str, &str)> = vec![
        ("content", content),
        ("name", &doc.name),
        ("agent", &doc.agent),
        ("role", &doc.role),
    ];
    if let Some(d) = &doc.description {
        props.push(("description", d));
    }
    record_entity_doc(
        store,
        index,
        prefix,
        doc.kind.as_str(),
        &doc.slug,
        &[
            ("name", &doc.name),
            ("agent", &doc.agent),
            ("role", &doc.role),
        ],
        &props,
        content,
        source,
        rel_path,
        candidates,
        written_relations,
        now,
    )
}

/// Graph-first authoring: record an artifact of any kind directly into
/// the graph, no source file. The kind's `fields` DSL extracts extra
/// properties; conformance is judged exactly as for captured files.
/// Rendering a projection from the recorded content is the caller's next
/// step for graph-first kinds.
pub fn author_artifact(
    store: &Store,
    prefix: &str,
    kind: &str,
    slug: &str,
    title: &str,
    content: &str,
    source: &str,
) -> Result<DocOutcome, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    let candidates = twinned_paths(store, prefix)?;
    let mut written = BTreeSet::new();
    let registry = crate::kinds::registry(store, &index)?;
    let mut props: Vec<(String, String)> = vec![
        ("content".to_string(), content.to_string()),
        ("title".to_string(), title.to_string()),
    ];
    if let Some(def) = registry.get(kind) {
        if let Some(rule) = def.rule() {
            for (p, v) in rule.extract(content, slug) {
                if p != "content" && p != "title" {
                    props.push((p, v));
                }
            }
        }
    }
    let prop_refs: Vec<(&str, &str)> = props
        .iter()
        .map(|(p, v)| (p.as_str(), v.as_str()))
        .collect();
    record_entity_doc(
        store,
        &index,
        prefix,
        kind,
        slug,
        &[("title", title)],
        &prop_refs,
        content,
        source,
        None,
        &candidates,
        &mut written,
        now_ms(),
    )
}

/// Explicit ingestion for documents outside the observed tree — the path
/// for Claude Code plan files (`~/.claude/plans/*.md`) or pasted decisions.
pub fn add_doc(
    store: &Store,
    prefix: &str,
    meta: &DocMeta,
    content: &str,
    source: &str,
) -> Result<DocOutcome, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    let candidates = twinned_paths(store, prefix)?;
    let mut written = BTreeSet::new();
    let out = record_doc(
        store,
        &index,
        prefix,
        meta,
        content,
        source,
        None,
        &candidates,
        &mut written,
        now_ms(),
    )?;
    // Re-registering a document is a statement that it is being worked
    // again: a plan re-added under a slug that was marked done would
    // otherwise stay invisible in every active list, with nothing saying
    // why. Supersession is left alone — it points at a successor, and
    // reviving past it would contradict that edge.
    if out.wrote {
        let sid = StableId::derive(&[meta.kind.as_str(), prefix, &meta.slug]);
        let (state, _) = crate::lifecycle::of(&index, store, &sid)?;
        use crate::lifecycle::Lifecycle;
        if matches!(
            state,
            Lifecycle::Done | Lifecycle::Abandoned | Lifecycle::Retired
        ) {
            crate::lifecycle::set(store, &index, &sid, Lifecycle::Active, Some("re-registered"))?;
        }
    }
    Ok(out)
}

/// Explicit ingestion for agent skills/configuration outside the observed
/// tree — user-level `~/.claude/skills`, `~/.claude/CLAUDE.md`, and the like.
pub fn add_agent_doc(
    store: &Store,
    prefix: &str,
    doc: &AgentDoc,
    content: &str,
    source: &str,
) -> Result<DocOutcome, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    let candidates = twinned_paths(store, prefix)?;
    let mut written = BTreeSet::new();
    record_agent_doc(
        store,
        &index,
        prefix,
        doc,
        content,
        source,
        None,
        &candidates,
        &mut written,
        now_ms(),
    )
}

/// Rel paths of files twinned under a prefix, from the namespace.
pub fn twinned_paths(store: &Store, prefix: &str) -> Result<BTreeSet<String>, StoreError> {
    Ok(store
        .namespace()?
        .keys()
        .filter_map(|n| n.strip_prefix(&format!("{prefix}/")))
        .map(str::to_string)
        .collect())
}
