//! "What is planned, what is moving, and what is done?"
//!
//! The one surface read down the spine rather than across a list of
//! kinds. A stage holds the features planned for it; a feature holds the
//! work in flight against it; and everything the graph cannot attribute
//! is shown rather than filed away.
//!
//! **A stage's state is never derived from its features.** A stage is a
//! body of work — a research question, in Stage 1's case — and four
//! finished features do not finish a question. The stage says what was
//! recorded about it; the features say what they can show. Reading them
//! together is the point, and collapsing them into one verdict would be
//! the invention this product exists to refuse.

use crate::dto::*;
use crate::query;
use crate::say;
use crate::state::Loaded;
use brain_core::ids::StableId;
use brain_observe::lifecycle;
use brain_observe::twin;
use std::collections::{BTreeMap, BTreeSet};

pub fn build(loaded: &Loaded) -> Result<RoadmapView, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let now = loaded.snapshot.generated_at_ms;

    // Everything unfinished, attributed through the files it touches.
    let mut inflight: Vec<InFlight> = Vec::new();
    for item in query::work::changes(loaded)? {
        inflight.push(from_work(item, "change"));
    }
    for item in query::work::plans(loaded)? {
        let kind = item.kind.clone();
        inflight.push(from_work(item, &kind));
    }
    for session in query::work::build(loaded)?.sessions {
        // Only what is still moving, or moved since the last consolidation.
        if !session.live {
            continue;
        }
        inflight.push(InFlight {
            id: session.id,
            kind: "agent_session".to_string(),
            noun: say::kind_noun("agent_session").to_string(),
            glyph: say::kind_glyph("agent_session").to_string(),
            title: session.objective,
            stage: session.state,
            note: format!("{}, {}", session.agent_label, session.ran_for),
            tone: "watch".to_string(),
            when: say::ago(now, session.at_ms),
            at_ms: session.at_ms,
            because: None,
            fix_command: None,
        });
    }

    // Which feature each in-flight item belongs to, and why.
    let mut by_feature: BTreeMap<String, Vec<InFlight>> = BTreeMap::new();
    let mut unattributed: Vec<InFlight> = Vec::new();
    for item in inflight {
        let owners = loaded.spine().features_of(&StableId(item.id.clone()));
        if owners.is_empty() {
            unattributed.push(item);
            continue;
        }
        for owned in owners {
            let through = owned
                .through
                .as_ref()
                .map(|file| twin::sid_label(index, store, file));
            let mut copy = item.clone();
            copy.because = Some(say::attribution_because(
                owned.via.as_str(),
                &owned.predicate,
                through.as_deref(),
            ));
            by_feature.entry(owned.slug.clone()).or_default().push(copy);
        }
    }

    // The newest change to the code a feature is built by — one reverse
    // pass over the shared event scan, not a scan per feature.
    //
    // Declared files only, deliberately. Taken over the whole reach, one
    // document that happens to mention every path becomes the newest
    // thing inside every feature, and the column answers the same for all
    // of them. "Last moved" has to mean the code.
    let mut owners_of_file: BTreeMap<&StableId, Vec<&str>> = BTreeMap::new();
    for slug in loaded.spine().slugs() {
        if let Some(reach) = loaded.spine().reach(slug) {
            for file in &reach.files {
                owners_of_file.entry(file).or_default().push(slug);
            }
        }
    }
    let mut last: BTreeMap<String, (u64, StableId)> = BTreeMap::new();
    for row in loaded.events().iter().rev() {
        let Some(subject) = &row.subject else { continue };
        for slug in owners_of_file.get(subject).into_iter().flatten() {
            last.entry((*slug).to_string())
                .or_insert_with(|| (row.at_ms, subject.clone()));
        }
    }

    let row_of = |slug: &str| -> Result<RoadmapRow, String> {
        let node = super::features::node(loaded, slug, 0)?;
        let (when, what) = match last.get(slug) {
            Some((at_ms, sid)) => (
                say::ago(now, *at_ms),
                Some(query::make_ref(index, store, sid)),
            ),
            None => (String::new(), None),
        };
        Ok(RoadmapRow {
            id: node.id,
            slug: node.slug.clone(),
            title: node.title,
            done: node.done,
            met: node.met,
            total: node.total,
            verdict: node.verdict,
            tone: node.tone,
            strip: node.strip,
            inflight: by_feature.get(&node.slug).cloned().unwrap_or_default(),
            last_touched: when,
            last_touched_what: what,
        })
    };

    // Stages, and the features planned for them.
    let mut stages: Vec<RoadmapStage> = Vec::new();
    let mut planned: BTreeSet<String> = BTreeSet::new();
    for (sid, labels) in query::scoped(index, store, prefix, "stage")? {
        let slug = labels.get("slug").cloned().unwrap_or_default();
        let mut features = Vec::new();
        for (_, from) in twin::live_to(index, store, &sid, "planned_for")
            .map_err(|e| e.to_string())?
        {
            let feature_labels = query::labels_of(index, store, &from);
            let Some(feature_slug) = feature_labels.get("slug") else {
                continue;
            };
            planned.insert(feature_slug.clone());
            features.push(row_of(feature_slug)?);
        }
        features.sort_by(|a, b| a.done.cmp(&b.done).then(a.title.cmp(&b.title)));

        let ready = features.iter().filter(|row| row.done).count();
        let total = features.len();
        let (state, why) = lifecycle::of(index, store, &sid).map_err(|e| e.to_string())?;
        let body = twin::latest(index, store, &sid, "content")
            .map_err(|e| e.to_string())?
            .unwrap_or_default();

        stages.push(RoadmapStage {
            id: sid.to_string(),
            title: query::display_name(index, store, &sid, &labels),
            summary: query::excerpt(&body, 220),
            state: say::lifecycle(state.as_str(), &why),
            tone: if total > 0 && ready == total {
                "good".to_string()
            } else {
                "quiet".to_string()
            },
            verdict: say::stage_verdict(ready, total),
            ready,
            total,
            features,
            slug,
        });
    }
    stages.sort_by(|a, b| a.slug.cmp(&b.slug));

    // Features no stage claims — directly or through a parent. A part
    // inherits the stage its whole is planned for; saying otherwise would
    // list every part of every planned feature as unplanned work.
    let mut unplanned = Vec::new();
    for (sid, labels) in query::scoped(index, store, prefix, "feature")? {
        let slug = labels.get("slug").cloned().unwrap_or_default();
        if slug.is_empty() || planned.contains(&slug) {
            continue;
        }
        let inherited = brain_observe::features::ancestry(store, index, &sid)
            .map_err(|e| e.to_string())?
            .iter()
            .any(|(_, parent)| planned.contains(parent));
        if inherited {
            continue;
        }
        unplanned.push(row_of(&slug)?);
    }
    unplanned.sort_by(|a, b| a.done.cmp(&b.done).then(a.title.cmp(&b.title)));

    let moving: usize = stages
        .iter()
        .flat_map(|stage| &stage.features)
        .map(|row| row.inflight.len())
        .sum::<usize>()
        + unattributed.len();

    Ok(RoadmapView {
        snapshot: loaded.snapshot.clone(),
        headline: say::roadmap_headline(stages.len(), moving),
        note: "A stage says what was recorded about it. Its features say what they can show. \
               Neither is derived from the other."
            .to_string(),
        stages,
        unplanned,
        unattributed,
    })
}

fn from_work(item: WorkItem, kind: &str) -> InFlight {
    InFlight {
        id: item.id,
        kind: kind.to_string(),
        noun: item.noun,
        glyph: item.glyph,
        title: item.title,
        stage: item.stage,
        note: item.note,
        tone: item.tone,
        when: item.when,
        at_ms: item.at_ms,
        because: None,
        fix_command: item.fix_command,
    }
}
