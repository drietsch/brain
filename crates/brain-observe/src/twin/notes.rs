//! What a person noticed, kept beside what the machine measured.

use brain_core::ids::{NodeId, StableId};
use brain_core::object::Object;
use brain_index::{Index, MemIndex};
use brain_store::{now_ms, Store, StoreError};

/// The classifications a note may carry. Dead ends are the expensive
/// knowledge: what was tried and failed, so no session walks it twice.
pub const NOTE_KINDS: &[&str] = &["learning", "dead-end", "gap", "decision-pending"];

/// A note's classification, when its text carries one (`[dead-end] ...`).
/// Encoding the kind in the value keeps every existing consumer working —
/// the tag reads as prose and parses as data.
pub fn note_kind(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix('[')?;
    let (kind, body) = rest.split_once("] ")?;
    NOTE_KINDS.contains(&kind).then_some((kind, body))
}

/// Add a classified note: `[kind] text`, with `kind` from [`NOTE_KINDS`].
pub fn add_note_kinded(
    store: &Store,
    subject: &StableId,
    kind: &str,
    text: &str,
) -> Result<NodeId, StoreError> {
    add_note(store, subject, &format!("[{kind}] {text}"))
}

pub fn add_note(store: &Store, subject: &StableId, text: &str) -> Result<NodeId, StoreError> {
    store.put(&Object::Observation {
        subject: subject.clone(),
        property: "note".to_string(),
        value: text.to_string(),
        source: "agent".to_string(),
        observed_at_ms: now_ms(),
    })
}

/// All notes on an entity, oldest first. Ordered by the event log rather
/// than by timestamp sorting: the log is chronological by construction, so
/// two notes written in the same millisecond keep their true order.
pub fn notes(
    index: &MemIndex,
    store: &Store,
    subject: &StableId,
) -> Result<Vec<(u64, String)>, StoreError> {
    // Sort the subject's own observations by their position in the feed,
    // rather than walking the feed looking for them. Same order, and it
    // costs the subject's handful of observations instead of the whole
    // log — which was being re-read and re-parsed once per subject.
    let order = store.put_position()?;
    let mut candidates: Vec<NodeId> = index.observations_of(subject);
    candidates.sort_by_key(|id| order.get(id).copied().unwrap_or(usize::MAX));

    let mut out = Vec::new();
    for id in candidates {
        if let Object::Observation {
            property,
            value,
            observed_at_ms,
            ..
        } = store.get(&id)?
        {
            if property == "note" {
                out.push((observed_at_ms, value));
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Decisions and plans: the *why* documents, as first-class twin entities
// ---------------------------------------------------------------------------
