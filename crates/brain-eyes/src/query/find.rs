//! "Take me to X."
//!
//! Structured search over what the graph knows: names, paths, titles and
//! slugs. No answer is composed — every hit says plainly why it matched
//! and opens the thing itself.

use crate::dto::*;
use crate::query;
use crate::say;
use crate::state::Loaded;
use brain_observe::{lifecycle, twin};
use std::collections::BTreeSet;

pub fn build(loaded: &Loaded, raw: &str, limit: usize) -> Result<FindView, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let query_text = raw.trim().to_string();
    if query_text.is_empty() {
        return Ok(FindView {
            snapshot: loaded.snapshot.clone(),
            query: query_text,
            hits: Vec::new(),
            note: None,
        });
    }
    let needle = query_text.to_lowercase();

    // Every kind the graph holds, plus files.
    let mut searchable: Vec<String> = loaded.registry().keys().cloned().collect();
    for extra in ["feature", "change", "test_case", "test_run", "template"] {
        if !searchable.iter().any(|kind| kind == extra) {
            searchable.push(extra.to_string());
        }
    }

    let mut scored: Vec<(u32, FindHit)> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    // Paths already represented by a richer entity (a decision, a plan),
    // so the file behind it does not appear as a second result.
    let mut covered_paths: BTreeSet<String> = BTreeSet::new();

    for kind in &searchable {
        for (sid, labels) in query::scoped(index, store, prefix, kind)? {
            let title = query::title_of(index, store, &sid, &labels);
            let label = twin::sid_label(index, store, &sid);
            let slug = labels.get("slug").cloned().unwrap_or_default();
            let Some((score, because)) = match_score(&needle, &title, &label, &slug) else {
                continue;
            };
            if !seen.insert(sid.to_string()) {
                continue;
            }
            if let Some(path) = labels.get("path") {
                covered_paths.insert(path.clone());
            }
            let (state, why) = lifecycle::of(index, store, &sid).map_err(|e| e.to_string())?;
            scored.push((
                score,
                FindHit {
                    target: query::make_ref(index, store, &sid),
                    because,
                    state: say::lifecycle(state.as_str(), &why),
                    features: query::features_of(loaded, &sid),
                },
            ));
        }
    }

    // Files: matched on their path.
    for (path, sid) in query::present_files(index, store, prefix)? {
        if !path.to_lowercase().contains(&needle) || covered_paths.contains(&path) {
            continue;
        }
        if !seen.insert(sid.to_string()) {
            continue;
        }
        let score = if path.to_lowercase() == needle { 100 } else { 60 };
        scored.push((
            score,
            FindHit {
                target: query::make_ref(index, store, &sid),
                because: "matches this file's path".to_string(),
                state: None,
                features: query::features_of(loaded, &sid),
            },
        ));
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.target.label.cmp(&b.1.target.label)));
    let total = scored.len();
    let hits: Vec<FindHit> = scored.into_iter().take(limit).map(|(_, hit)| hit).collect();
    let note = (total > hits.len()).then(|| {
        format!(
            "showing {} of {}",
            hits.len(),
            say::count(total as u64, "match", "matches")
        )
    });

    Ok(FindView {
        snapshot: loaded.snapshot.clone(),
        query: query_text,
        hits,
        note,
    })
}

fn match_score(needle: &str, title: &str, label: &str, slug: &str) -> Option<(u32, String)> {
    let lower_title = title.to_lowercase();
    let lower_label = label.to_lowercase();
    let lower_slug = slug.to_lowercase();
    if lower_title == needle || lower_slug == needle {
        return Some((100, "exact name".to_string()));
    }
    if lower_title.starts_with(needle) {
        return Some((90, "title starts with this".to_string()));
    }
    if lower_title.contains(needle) {
        return Some((70, "mentioned in the title".to_string()));
    }
    if lower_slug.contains(needle) || lower_label.contains(needle) {
        return Some((50, "matches its name".to_string()));
    }
    None
}
