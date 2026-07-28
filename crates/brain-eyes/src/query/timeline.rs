//! "What happened?" — as episodes, not events.
//!
//! A refresh writes every fact it observed with one timestamp (that is
//! already how co-change is detected in `brain_observe::assoc`), so the
//! event log arrives pre-grouped: each batch is one thing that happened.
//! Eyes titles the batch and lists what it touched, instead of printing
//! two hundred rows of "file changed · content identity e65b66e…".

use crate::dto::*;
use crate::query;
use crate::say;
use crate::state::{EventPayload, EventRow, Loaded};
use brain_core::ids::StableId;
use std::collections::BTreeSet;

pub fn build(loaded: &Loaded, limit: usize) -> Result<TimelineView, String> {
    Ok(TimelineView {
        snapshot: loaded.snapshot.clone(),
        episodes: episodes_since(loaded, 0, limit)?,
    })
}

/// The most recent episodes after `since`, newest first.
pub fn episodes_since(
    loaded: &Loaded,
    since: u64,
    limit: usize,
) -> Result<Vec<Episode>, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let now = loaded.snapshot.generated_at_ms;

    // Group by exact observation time: one refresh, one run, one change.
    let mut batches: Vec<(u64, Vec<&EventRow>)> = Vec::new();
    for row in loaded.events() {
        if row.at_ms <= since {
            continue;
        }
        match batches.last_mut() {
            Some((at, rows)) if *at == row.at_ms => rows.push(row),
            _ => batches.push((row.at_ms, vec![row])),
        }
    }
    batches.sort_by(|a, b| b.0.cmp(&a.0));

    let mut out = Vec::new();
    for (at_ms, rows) in batches.into_iter().take(limit * 3) {
        if out.len() >= limit {
            break;
        }
        if let Some(episode) = describe(loaded, at_ms, &rows, now, prefix)? {
            out.push(episode);
        }
    }
    let _ = (store, index);
    Ok(out)
}

fn describe(
    loaded: &Loaded,
    at_ms: u64,
    rows: &[&EventRow],
    now: u64,
    prefix: &str,
) -> Result<Option<Episode>, String> {
    let store = &loaded.store;
    let index = &loaded.index;

    let mut changed_files: Vec<StableId> = Vec::new();
    let mut new_files = 0usize;
    let mut docs: BTreeSet<String> = BTreeSet::new();
    let mut gone = 0usize;
    let mut consolidated: Option<String> = None;
    let mut run: Option<String> = None;
    let mut change_status: Option<(String, String)> = None;
    let mut stale_marks = 0usize;
    let mut acks = 0usize;
    let mut effect: Option<String> = None;

    for row in rows {
        match &row.payload {
            EventPayload::Observation { property, value } => {
                let Some(subject) = &row.subject else { continue };
                match property.as_str() {
                    "content_b3" => changed_files.push(subject.clone()),
                    "present" if value == "false" => gone += 1,
                    "present" if value == "true" => new_files += 1,
                    "content" => {
                        if let Some(kind) = query::kind_of(index, store, subject) {
                            if kind != "source_file" {
                                docs.insert(query::title_of(
                                    index,
                                    store,
                                    subject,
                                    &query::labels_of(index, store, subject),
                                ));
                            }
                        }
                    }
                    "session_summary" => consolidated = Some(value.clone()),
                    "reviewed" => acks += 1,
                    "conforms" if value == "false" => stale_marks += 1,
                    "passed" | "total" => {}
                    "status" => {
                        if query::kind_of(index, store, subject).as_deref() == Some("change") {
                            let labels = query::labels_of(index, store, subject);
                            change_status = Some((
                                value.clone(),
                                labels
                                    .get("slug")
                                    .cloned()
                                    .unwrap_or_else(|| "a change".to_string()),
                            ));
                        }
                    }
                    _ => {}
                }
            }
            EventPayload::Receipt { ok, detail } => {
                effect = Some(if *ok {
                    format!("effect confirmed: {detail}")
                } else {
                    format!("effect failed: {detail}")
                });
            }
            _ => {}
        }
    }

    // Test runs arrive as a test_run entity plus per-case results; the
    // insights totals are the readable form.
    if run.is_none() {
        let totals = rows.iter().filter_map(|row| match &row.payload {
            EventPayload::Observation { property, value } if property == "total" => {
                value.parse::<usize>().ok()
            }
            _ => None,
        });
        if let Some(total) = totals.max() {
            let failed = rows
                .iter()
                .filter_map(|row| match &row.payload {
                    EventPayload::Observation { property, value } if property == "failed" => {
                        value.parse::<usize>().ok()
                    }
                    _ => None,
                })
                .max()
                .unwrap_or(0);
            run = Some(if failed == 0 {
                format!("all {total} tests passed")
            } else {
                format!("{failed} of {total} tests failed")
            });
        }
    }

    let when = say::ago(now, at_ms);
    let mut facts: Vec<String> = Vec::new();
    let mut items: Vec<Ref> = Vec::new();
    let total_changed = changed_files.len();

    for sid in changed_files.iter().take(6) {
        items.push(query::make_ref(index, store, sid));
    }

    let (kind, title) = if let Some(summary) = consolidated {
        facts.push(say::session_summary(&summary));
        ("session".to_string(), "Session consolidated".to_string())
    } else if let Some((status, slug)) = change_status {
        let (stage, note) = say::change_stage(&status);
        facts.push(note.to_string());
        if let Some(effect) = effect.clone() {
            facts.push(effect);
        }
        (
            "change".to_string(),
            format!("Governed change {slug} {stage}"),
        )
    } else if let Some(run) = run {
        ("tests".to_string(), capitalize(&run))
    } else if total_changed > 0 || new_files > 0 || !docs.is_empty() {
        let mut parts = Vec::new();
        if total_changed > 0 {
            parts.push(format!(
                "{} changed",
                say::count(total_changed as u64, "file", "files")
            ));
        }
        if new_files > 0 {
            parts.push(format!(
                "{} appeared",
                say::count(new_files as u64, "file", "files")
            ));
        }
        if gone > 0 {
            parts.push(format!(
                "{} removed",
                say::count(gone as u64, "file", "files")
            ));
        }
        if !docs.is_empty() {
            let names: Vec<String> = docs.iter().take(3).cloned().collect();
            facts.push(format!("documents updated: {}", names.join(", ")));
        }
        if stale_marks > 0 {
            facts.push(format!(
                "{} stopped matching its contract",
                say::count(stale_marks as u64, "document", "documents")
            ));
        }
        if acks > 0 {
            facts.push(format!(
                "{} reviewed and confirmed still accurate",
                say::count(acks as u64, "document was", "documents were")
            ));
        }
        if parts.is_empty() {
            return Ok(None);
        }
        ("observation".to_string(), capitalize(&parts.join(", ")))
    } else if acks > 0 {
        (
            "review".to_string(),
            format!(
                "{} reviewed and confirmed still accurate",
                say::count(acks as u64, "document was", "documents were")
            ),
        )
    } else {
        return Ok(None);
    };

    let _ = prefix;
    Ok(Some(Episode {
        at_ms,
        when,
        kind,
        title,
        facts,
        more: total_changed.saturating_sub(items.len()),
        items,
    }))
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}
