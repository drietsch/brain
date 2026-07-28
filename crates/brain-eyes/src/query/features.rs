//! "What do we claim, and what actually backs it?" — as a tree.
//!
//! A real feature is rarely one claim. It has a core, an API, a user
//! interface, tests, documentation, and each of those is separately
//! buildable and provable. The graph records that with `part_of`; this
//! turns it into something a person can scan, filter and open.
//!
//! Every feature carries a **dimension strip**: one cell per part, or —
//! for a feature with no parts — one cell per requirement. The strip is
//! the same object at every scale, from seven pixels in a list row to a
//! labelled row of bars in a dossier, and each cell is a shape as well as
//! a colour so nothing depends on colour alone.

use crate::dto::*;
use crate::query;
use crate::say;
use crate::state::Loaded;
use brain_core::ids::StableId;
use brain_observe::features::{self, DoneReport, PartReport};
use brain_observe::twin;
use std::collections::BTreeSet;

pub fn build(loaded: &Loaded) -> Result<FeaturesView, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();

    // Roots are the features nothing else claims as a part.
    let mut roots = Vec::new();
    for (sid, labels) in query::scoped(index, store, prefix, "feature")? {
        if features::parent(store, index, &sid)
            .map_err(|e| e.to_string())?
            .is_some()
        {
            continue;
        }
        let slug = labels.get("slug").cloned().unwrap_or_default();
        roots.push(node(loaded, &slug, 0)?);
    }
    roots.sort_by(|a, b| {
        // Unfinished first — a list of features is a list of work.
        a.done.cmp(&b.done).then(a.title.cmp(&b.title))
    });

    let mut dimensions: BTreeSet<String> = BTreeSet::new();
    collect_dimensions(&roots, &mut dimensions);

    let total = count_all(&roots);
    let ready = count_ready(&roots);
    let headline = if total == 0 {
        "No features are registered yet.".to_string()
    } else if ready == total {
        format!(
            "All {} are ready.",
            say::count(total as u64, "feature", "features")
        )
    } else {
        format!("{ready} of {total} features are ready.")
    };

    Ok(FeaturesView {
        snapshot: loaded.snapshot.clone(),
        headline,
        note: "A feature with parts is judged by its parts — readiness rolls up from the leaves, and is never set by hand.".to_string(),
        roots,
        dimensions: dimensions.into_iter().collect(),
        coverage: coverage(loaded),
    })
}

fn count_all(nodes: &[FeatureNode]) -> usize {
    nodes.iter().map(|n| 1 + count_all(&n.parts)).sum()
}
fn count_ready(nodes: &[FeatureNode]) -> usize {
    nodes
        .iter()
        .map(|n| usize::from(n.done) + count_ready(&n.parts))
        .sum()
}
fn collect_dimensions(nodes: &[FeatureNode], out: &mut BTreeSet<String>) {
    for node in nodes {
        for cell in &node.strip {
            out.insert(cell.label.clone());
        }
        collect_dimensions(&node.parts, out);
    }
}

/// One feature, with its strip and its parts.
pub fn node(loaded: &Loaded, slug: &str, depth: usize) -> Result<FeatureNode, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let now = loaded.snapshot.generated_at_ms;

    let sid = features::feature_sid(prefix, slug);
    let labels = query::labels_of(index, store, &sid);
    let report = features::evaluate(store, index, prefix, slug).map_err(|e| e.to_string())?;
    let (met, total) = report.score();

    let strip = strip(loaded, &sid, &report)?;
    let mut parts = Vec::new();
    for part in &report.parts {
        parts.push(node(loaded, &part.slug, depth + 1)?);
    }

    let verdict = verdict(&report, met, total);
    let tone = if report.done {
        "good"
    } else if met == 0 {
        "bad"
    } else {
        "watch"
    };

    let at_ms = query::changed_at(index, store, &sid);
    Ok(FeatureNode {
        id: sid.to_string(),
        slug: slug.to_string(),
        title: query::display_name(index, store, &sid, &labels),
        status: twin::latest(index, store, &sid, "status")
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| "planned".to_string()),
        done: report.done,
        met,
        total,
        by_parts: report.by_parts(),
        blocked_by: report.blocked_by.clone(),
        verdict,
        tone: tone.to_string(),
        strip,
        parts,
        depth,
        when: if at_ms > 0 {
            say::ago(now, at_ms)
        } else {
            String::new()
        },
        at_ms,
    })
}

fn verdict(report: &DoneReport, met: usize, total: usize) -> String {
    if report.by_parts() {
        return match (&report.blocked_by, report.done) {
            (_, true) => format!(
                "every one of its {} is ready",
                say::count(total as u64, "part", "parts")
            ),
            (Some(blocking), _) => format!("{met} of {total} parts ready — waiting on {blocking}"),
            (None, _) => format!("{met} of {total} parts ready"),
        };
    }
    if report.done {
        return "every requirement is met".to_string();
    }
    if met == 0 {
        return "nothing is linked to it yet".to_string();
    }
    format!("{met} of {total} requirements met")
}

/// The strip: a feature's parts, or its requirements when it has none.
fn strip(
    loaded: &Loaded,
    sid: &StableId,
    report: &DoneReport,
) -> Result<Vec<StripCell>, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();

    if report.by_parts() {
        return Ok(report
            .parts
            .iter()
            .map(|part| part_cell(prefix, part))
            .collect());
    }

    let mut out = Vec::new();
    for check in &report.checks {
        let label = say::dod_label(&check.predicate).to_string();
        if check.count == 0 {
            out.push(StripCell {
                detail: format!("nothing is linked as {label}"),
                label,
                state: "absent".to_string(),
                id: None,
                records: Vec::new(),
            });
            continue;
        }
        // A requirement is only *ready* when everything linked to it still
        // stands. Linked-but-unestablished is its own state, not a pass.
        //
        // Every linked record is resolved, not merely up to the first bad
        // one: the cell carries them all, so what the verdict is about can
        // be opened. A cell that hid half its evidence would be the exact
        // failure this surface exists to prevent.
        let linked = twin::live_from(index, store, sid, &check.predicate)
            .map_err(|e| e.to_string())?;
        let mut worst = "ready";
        let mut detail = format!(
            "{} linked",
            say::count(check.count as u64, "record", "records")
        );
        let mut records = Vec::new();
        for (_, to) in &linked {
            let target = query::make_ref(index, store, to);
            let (text, basis, tone) = super::evidence::resolve_link(loaded, to, &label, &target)?;
            match tone.as_str() {
                "bad" if worst != "failing" => {
                    worst = "failing";
                    detail = text.clone();
                }
                "watch" if worst == "ready" => {
                    worst = "unproven";
                    detail = text.clone();
                }
                _ => {}
            }
            records.push(StripRecord {
                target,
                text,
                basis,
                tone,
            });
        }
        out.push(StripCell {
            label,
            state: worst.to_string(),
            detail,
            id: None,
            records,
        });
    }
    Ok(out)
}

fn part_cell(prefix: &str, part: &PartReport) -> StripCell {
    let (state, detail) = if part.done {
        ("ready", format!("{} is ready", part.title))
    } else if part.met == 0 {
        ("absent", format!("nothing is linked to {} yet", part.title))
    } else {
        (
            "unproven",
            format!("{} is {} of {}", part.title, part.met, part.total),
        )
    };
    StripCell {
        label: part.title.clone(),
        state: state.to_string(),
        detail,
        // A part is an entity, so its cell opens it directly.
        id: Some(features::feature_sid(prefix, &part.slug).to_string()),
        records: Vec::new(),
    }
}

/// How much of the graph any feature reaches at all.
///
/// Silent until some feature declares something: on a graph with no
/// spine, "nothing is claimed" is true, useless, and exactly the noise
/// absence-is-silence exists to prevent.
fn coverage(loaded: &Loaded) -> Option<SpineCensus> {
    let spine = loaded.spine();
    if !spine.asked() {
        return None;
    }
    let index = &loaded.index;
    let store = &loaded.store;
    let rows: Vec<CoverageRow> = spine
        .census()
        .iter()
        .map(|row| {
            let missing = spine.unclaimed(&row.kind);
            CoverageRow {
                label: say::kind_plural(&row.kind),
                glyph: say::kind_glyph(&row.kind).to_string(),
                claimed: row.claimed,
                total: row.total,
                tone: match () {
                    // A file belonging to no feature is ordinary; a
                    // document belonging to none is worth a second look.
                    _ if row.kind == "source_file" => "quiet".to_string(),
                    _ if row.claimed == row.total => "good".to_string(),
                    _ if row.claimed == 0 => "watch".to_string(),
                    _ => "quiet".to_string(),
                },
                note: say::coverage_note(&row.kind, row.claimed, row.total),
                unclaimed: missing
                    .iter()
                    .take(6)
                    .map(|sid| query::make_ref(index, store, sid))
                    .collect(),
                unclaimed_total: missing.len(),
                kind: row.kind.clone(),
            }
        })
        .collect();
    let (claimed, total) = spine.claimed_total();
    Some(SpineCensus {
        claimed,
        total,
        sentence: say::coverage_sentence(claimed, total),
        rows,
    })
}
