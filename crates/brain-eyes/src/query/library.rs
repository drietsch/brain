//! "Show me everything the brain knows, so I can read it."
//!
//! Shelves, not tables. The shape of the content decides the shape of the
//! view: a decision is a reading list entry with its status and what it
//! governs; a feature is a coverage strip; a test protocol is a result.
//! Every shelf is built from the kind registry, so a kind taught at
//! runtime gets a shelf without a code change.

use crate::dto::*;
use crate::query;
use crate::say;
use crate::state::Loaded;
use brain_observe::{features, lifecycle, twin};
use std::collections::{BTreeMap, BTreeSet};

/// Shelf id → (label, note, kinds it gathers).
const SHELVES: &[(&str, &str, &str, &[&str])] = &[
    (
        "decisions",
        "Decisions",
        "Why the system is the way it is.",
        &["decision"],
    ),
    (
        "plans",
        "Plans",
        "Work that was written down before it was done.",
        &["plan", "task_list"],
    ),
    (
        "documents",
        "Documents",
        "Prose that is meant to track the code.",
        &["doc", "runbook"],
    ),
    (
        "features",
        "Features",
        "What the system claims to do, and what backs the claim.",
        &["feature"],
    ),
    (
        "changes",
        "Changes",
        "Edits the brain made to the workspace, with their receipts.",
        &["change"],
    ),
    (
        "agents",
        "Agent rules",
        "The instructions and skills agents read before working here.",
        &["agent_config", "skill"],
    ),
    (
        "assets",
        "Assets",
        "Images, prototypes and generated artefacts.",
        &["asset", "prototype", "capability_matrix"],
    ),
];

pub fn shelves(loaded: &Loaded) -> Result<Vec<Shelf>, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let mut out = Vec::new();
    for (id, label, note, kinds) in SHELVES {
        let mut count = 0usize;
        for kind in *kinds {
            count += query::scoped(index, store, prefix, kind)?.len();
        }
        if count == 0 && *id != "decisions" {
            continue; // absence is silence
        }
        out.push(Shelf {
            id: id.to_string(),
            label: label.to_string(),
            note: note.to_string(),
            count,
        });
    }
    // Pictures and recordings get their own surface: a screenshot is
    // something you look at, not a row in a list.
    let media = query::scoped(index, store, prefix, "asset")?
        .into_iter()
        .filter(|(sid, _)| {
            matches!(
                twin::latest(index, store, sid, "subtype")
                    .ok()
                    .flatten()
                    .as_deref(),
                Some("image") | Some("screencast") | Some("audio")
            )
        })
        .count();
    if media > 0 {
        out.push(Shelf {
            id: "media".to_string(),
            label: "Pictures & recordings".to_string(),
            note: "Screenshots, screencasts, and the narrated tour.".to_string(),
            count: media,
        });
    }

    // Kinds taught at runtime that no fixed shelf gathers.
    let known: BTreeSet<&str> = SHELVES.iter().flat_map(|(_, _, _, k)| k.iter().copied()).collect();
    for kind in loaded.registry().keys() {
        if known.contains(kind.as_str()) || kind == "template" {
            continue;
        }
        let count = query::scoped(index, store, prefix, kind)?.len();
        if count == 0 {
            continue;
        }
        out.push(Shelf {
            id: kind.clone(),
            label: capitalize_words(say::kind_noun(kind)),
            note: format!("Records of kind {}.", say::kind_noun(kind)),
            count,
        });
    }
    Ok(out)
}

pub fn build(loaded: &Loaded, shelf: &str, query_text: &str) -> Result<LibraryView, String> {
    let shelves = shelves(loaded)?;
    let shelf = if shelves.iter().any(|s| s.id == shelf) {
        shelf.to_string()
    } else {
        shelves
            .first()
            .map(|s| s.id.clone())
            .unwrap_or_else(|| "decisions".to_string())
    };
    let (label, note, kinds) = shelf_kinds(&shelf);
    // One insights pass for the whole shelf, not one per item.
    let stale = stale_map(loaded)?;
    let mut items = Vec::new();
    for kind in kinds {
        items.extend(items_for_kind(loaded, &kind, &stale)?);
    }

    if !query_text.trim().is_empty() {
        let needle = query_text.trim().to_lowercase();
        items.retain(|item| {
            item.title.to_lowercase().contains(&needle)
                || item.label.to_lowercase().contains(&needle)
                || item
                    .excerpt
                    .as_deref()
                    .is_some_and(|text| text.to_lowercase().contains(&needle))
        });
    }
    items.sort_by(|a, b| b.at_ms.cmp(&a.at_ms).then(a.title.cmp(&b.title)));

    Ok(LibraryView {
        snapshot: loaded.snapshot.clone(),
        shelves,
        shelf,
        label,
        note,
        items,
    })
}

fn shelf_kinds(shelf: &str) -> (String, String, Vec<String>) {
    for (id, label, note, kinds) in SHELVES {
        if *id == shelf {
            return (
                label.to_string(),
                note.to_string(),
                kinds.iter().map(|k| k.to_string()).collect(),
            );
        }
    }
    (
        capitalize_words(say::kind_noun(shelf)),
        String::new(),
        vec![shelf.to_string()],
    )
}

fn items_for_kind(
    loaded: &Loaded,
    kind: &str,
    stale: &StaleMap,
) -> Result<Vec<ShelfItem>, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let now = loaded.snapshot.generated_at_ms;

    let mut out = Vec::new();
    for (sid, labels) in query::scoped(index, store, prefix, kind)? {
        let (state, why) = lifecycle::of(index, store, &sid).map_err(|e| e.to_string())?;
        let at_ms = query::changed_at(index, store, &sid);
        let title = query::display_name(index, store, &sid, &labels);
        let slug = labels.get("slug").cloned().unwrap_or_default();

        let mut facts: Vec<String> = Vec::new();
        let mut coverage = None;
        let mut tone;
        let mut state_text;
        let mut state_note;

        // Lifecycle first: a finished plan is not a stale plan.
        if let Some(sentence) = say::lifecycle(state.as_str(), &why) {
            state_text = Some(sentence);
            state_note = None;
            tone = "quiet".to_string();
        } else {
            let entry = stale.get(&(kind.to_string(), slug.clone()));
            let (word, note) =
                say::freshness(entry.map(|(severity, _)| severity.as_str()), state.is_active());
            state_text = Some(word.to_string());
            state_note = Some(note.to_string());
            tone = match word {
                "may be wrong" => "watch",
                "current" => "good",
                _ => "quiet",
            }
            .to_string();
            if let Some((_, changed)) = entry {
                if !changed.is_empty() {
                    state_note = Some(format!("changed since: {}", changed.join(", ")));
                }
            }
        }

        match kind {
            "decision" => {
                if let Some(status) = twin::latest(index, store, &sid, "status")
                    .map_err(|e| e.to_string())?
                {
                    facts.push(format!("status: {status}"));
                }
                let governs = twin::live_from(index, store, &sid, "mentions")
                    .map_err(|e| e.to_string())?
                    .len();
                if governs > 0 {
                    facts.push(format!("governs {}", say::count(governs as u64, "file", "files")));
                }
            }
            "feature" => {
                let report = features::evaluate(store, index, prefix, &slug)
                    .map_err(|e| e.to_string())?;
                let cells: Vec<CoverageCell> = report
                    .checks
                    .iter()
                    .map(|check| CoverageCell {
                        label: say::dod_label(&check.predicate).to_string(),
                        met: check.count > 0,
                        detail: if check.count > 0 {
                            format!(
                                "{} linked",
                                say::count(check.count as u64, "record", "records")
                            )
                        } else {
                            format!("nothing linked as {}", say::dod_label(&check.predicate))
                        },
                    })
                    .collect();
                let met = cells.iter().filter(|c| c.met).count();
                state_text = Some(if report.done {
                    "complete".to_string()
                } else {
                    format!("{met} of {} in place", cells.len())
                });
                state_note = Some(
                    cells
                        .iter()
                        .filter(|c| !c.met)
                        .map(|c| format!("not {}", c.label))
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                tone = if report.done { "good" } else { "watch" }.to_string();
                coverage = Some(cells);
                if let Some(status) = twin::latest(index, store, &sid, "status")
                    .map_err(|e| e.to_string())?
                {
                    facts.push(format!("registered as {status}"));
                }
            }
            "change" => {
                let status = twin::latest(index, store, &sid, "status")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default();
                let (stage, note) = say::change_stage(&status);
                state_text = Some(stage.to_string());
                state_note = Some(note.to_string());
                tone = match status.as_str() {
                    "verified" | "reverted" => "good",
                    "broken" | "failed" | "indeterminate" => "bad",
                    _ => "watch",
                }
                .to_string();
                if let Some(target) = labels.get("target") {
                    facts.push(format!("touches {target}"));
                }
                if let Some(reason) = twin::latest(index, store, &sid, "reason")
                    .map_err(|e| e.to_string())?
                {
                    facts.push(reason);
                }
            }
            "asset" | "prototype" => {
                if let Some(subtype) = twin::latest(index, store, &sid, "subtype")
                    .map_err(|e| e.to_string())?
                {
                    facts.push(subtype);
                }
                let depicts = twin::live_from(index, store, &sid, "depicts")
                    .map_err(|e| e.to_string())?;
                if !depicts.is_empty() {
                    let names: Vec<String> = depicts
                        .iter()
                        .take(2)
                        .map(|(_, to)| twin::sid_label(index, store, to))
                        .collect();
                    facts.push(format!("shows {}", names.join(", ")));
                }
            }
            "skill" | "agent_config" => {
                if let Some(agent) = twin::latest(index, store, &sid, "agent")
                    .map_err(|e| e.to_string())?
                {
                    facts.push(format!("for {agent}"));
                }
                if let Some(role) = twin::latest(index, store, &sid, "role")
                    .map_err(|e| e.to_string())?
                {
                    facts.push(role);
                }
            }
            _ => {}
        }

        if let Some(path) = labels.get("path") {
            facts.push(path.clone());
        }

        let excerpt = twin::latest(index, store, &sid, "content")
            .map_err(|e| e.to_string())?
            .map(|text| query::excerpt(&text, 180))
            .filter(|text| !text.is_empty());

        out.push(ShelfItem {
            id: sid.to_string(),
            label: twin::sid_label(index, store, &sid),
            title,
            kind: kind.to_string(),
            noun: say::kind_noun(kind).to_string(),
            glyph: say::kind_glyph(kind).to_string(),
            state: state_text,
            state_note,
            tone,
            when: (at_ms > 0).then(|| say::ago(now, at_ms)),
            at_ms,
            facts,
            coverage,
            results: None,
            excerpt,
        });
    }
    Ok(out)
}

/// (kind, slug) → (severity, the files that changed underneath it).
type StaleMap = BTreeMap<(String, String), (String, Vec<String>)>;

fn stale_map(loaded: &Loaded) -> Result<StaleMap, String> {
    let insights = loaded.insights();
    Ok(insights
        .stale_docs
        .iter()
        .map(|doc| {
            (
                (doc.kind.clone(), doc.slug.clone()),
                (doc.severity.as_str().to_string(), doc.changed.clone()),
            )
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Concepts: the brain explaining its own vocabulary
// ---------------------------------------------------------------------------

pub fn concepts(loaded: &Loaded) -> Result<ConceptsView, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let fitness_rows = loaded.fitness();

    let mut out = Vec::new();
    for (kind, def) in loaded.registry() {
        let count = query::scoped(index, store, prefix, kind)?.len();
        let purpose = def
            .template
            .as_ref()
            .and_then(|sid| twin::latest(index, store, sid, "title").ok().flatten())
            .unwrap_or_else(|| capitalize_words(say::kind_noun(kind)));

        let placement_note = match def.placement.as_str() {
            "graph_first" => "written through brain; the file beside it is a read-only render",
            "projection" => "generated from a query; never written by hand",
            _ => "written as a file; brain observes it where it lies",
        };
        let enforcement_note = if def.enforce == "enforced" {
            "writes that do not meet the contract are refused"
        } else {
            "conformance is recorded, never blocked"
        };
        let rot_note = match twin::rot_severity(&def.rot, kind) {
            Some(twin::Severity::Warn) => "expected to track the code; goes stale when it drifts",
            Some(twin::Severity::Info) => "kept as a record; ages quietly",
            None => "never goes stale",
        };
        let verdicts = fitness_rows
            .iter()
            .find(|row| row.kind == *kind)
            .map(|row| row.verdicts.clone())
            .unwrap_or_default();

        out.push(Concept {
            kind: kind.clone(),
            label: capitalize_words(say::kind_noun(kind)),
            noun: say::kind_noun(kind).to_string(),
            glyph: say::kind_glyph(kind).to_string(),
            purpose,
            requires: def.requires.clone(),
            home: if def.project_to.is_empty() {
                def.home.clone()
            } else {
                vec![def.project_to.clone()]
            },
            placement: def.placement.clone(),
            placement_note: placement_note.to_string(),
            enforcement: def.enforce.clone(),
            enforcement_note: enforcement_note.to_string(),
            rot_note: rot_note.to_string(),
            count,
            verdicts,
        });
    }
    out.sort_by(|a, b| b.count.cmp(&a.count).then(a.label.cmp(&b.label)));
    Ok(ConceptsView {
        snapshot: loaded.snapshot.clone(),
        concepts: out,
    })
}

fn capitalize_words(text: &str) -> String {
    let mut out = String::new();
    for (index, word) in text.split(' ').enumerate() {
        if index > 0 {
            out.push(' ');
        }
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            if index == 0 {
                out.extend(first.to_uppercase());
            } else {
                out.push(first);
            }
            out.push_str(chars.as_str());
        }
    }
    out
}
