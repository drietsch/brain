//! Find: "where is the thing that does X" — lexical match over what the
//! twin already indexes (paths, symbol names, decision and plan titles,
//! notes), ranked with the graph's own centrality so a hub outranks a
//! leaf. The pre-embeddings answer to semantic search: no new index, no
//! new storage, one pass over derived projections.

use crate::twin::{self, latest};
use brain_core::object::Object;
use brain_index::{Index, MemIndex};
use brain_store::{Store, StoreError};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

#[derive(Serialize, Debug)]
pub struct Hit {
    pub label: String,
    /// file | decision | plan
    pub kind: String,
    pub score: u32,
    pub why: Vec<String>,
}

/// Rank everything under `prefix` against a free-text query.
pub fn find(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    query: &str,
) -> Result<Vec<Hit>, StoreError> {
    let terms: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .filter(|t| t.len() >= 2)
        .map(str::to_string)
        .collect();
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let ins = twin::insights_with(store, index, prefix)?;
    let hubs: BTreeMap<&String, usize> = ins.hubs.iter().map(|(p, n)| (p, *n)).collect();

    // Live files under the prefix — deleted files stay findable in
    // history, not in search results.
    let mut live: BTreeSet<String> = BTreeSet::new();
    for (name, node) in store.namespace()? {
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
        if latest(index, store, &id, "present")?.as_deref() == Some("false") {
            continue;
        }
        live.insert(rel.to_string());
    }

    struct Acc {
        kind: String,
        score: u32,
        why: Vec<String>,
    }
    let mut acc: BTreeMap<String, Acc> = BTreeMap::new();
    let mut bump = |label: &str, kind: &str, pts: u32, why: String| {
        let entry = acc.entry(label.to_string()).or_insert_with(|| Acc {
            kind: kind.to_string(),
            score: 0,
            why: Vec::new(),
        });
        entry.score += pts;
        if !entry.why.contains(&why) {
            entry.why.push(why);
        }
    };

    let matches = |haystack: &str| -> usize {
        let lower = haystack.to_lowercase();
        terms.iter().filter(|t| lower.contains(t.as_str())).count()
    };

    for rel in &live {
        let m = matches(rel);
        if m > 0 {
            let pts = 10 * m as u32 + if m == terms.len() { 5 } else { 0 };
            bump(rel, "file", pts, "path matches".to_string());
        }
    }

    // Symbol names attach to the file that declares them.
    for node in index.entities_by_kind("symbol") {
        let Ok(Object::Entity { labels, .. }) = store.get(&node) else {
            continue;
        };
        let (Some(file), Some(name)) = (labels.get("file"), labels.get("name")) else {
            continue;
        };
        if !live.contains(file) {
            continue;
        }
        let m = matches(name);
        if m > 0 {
            bump(file, "file", 15 * m as u32, format!("declares {name}"));
        }
    }

    for (slug, title, _status) in &ins.decisions {
        let m = matches(slug).max(matches(title));
        if m > 0 {
            bump(slug, "decision", 12 * m as u32, format!("decision: {title}"));
        }
    }
    for (slug, title) in &ins.plans {
        let m = matches(slug).max(matches(title));
        if m > 0 {
            bump(slug, "plan", 12 * m as u32, format!("plan: {title}"));
        }
    }

    // Notes vouch for their subject — past sessions talked about it here.
    for (_, entity, text) in &ins.notes {
        let m = matches(text);
        if m > 0 {
            let rel = entity
                .strip_prefix(&format!("{prefix}/"))
                .unwrap_or(entity);
            bump(rel, "file", 6 * m as u32, "a note mentions it".to_string());
        }
    }

    // Centrality: a hub that matches outranks a leaf that matches.
    for (label, a) in acc.iter_mut() {
        if let Some(n) = hubs.get(label) {
            a.score += (*n).min(10) as u32;
            a.why.push(format!("hub {n}"));
        }
    }

    let mut hits: Vec<Hit> = acc
        .into_iter()
        .map(|(label, a)| Hit {
            label,
            kind: a.kind,
            score: a.score,
            why: a.why,
        })
        .collect();
    hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.label.cmp(&b.label)));
    Ok(hits)
}

/// The textual projection of the hits.
pub fn render(hits: &[Hit], query: &str, prefix: &str, top: usize) -> String {
    let mut out = String::new();
    writeln!(out, "== find: \"{query}\" under {prefix} ==").ok();
    if hits.is_empty() {
        write!(
            out,
            "no matches — the twin searches paths, symbol names, decision and plan titles, and notes"
        )
        .ok();
        return out;
    }
    for (i, h) in hits.iter().take(top).enumerate() {
        writeln!(
            out,
            "{:>2}. [{:>3}] {} ({}) — {}",
            i + 1,
            h.score,
            h.label,
            h.kind,
            h.why.join(", ")
        )
        .ok();
    }
    if hits.len() > top {
        writeln!(out, "  … {} more (--top {})", hits.len() - top, hits.len()).ok();
    }
    write!(out, "then: brain before <name> | brain related <name>").ok();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::twin::refresh;
    use brain_index::replay;
    use std::fs;

    #[test]
    fn find_ranks_symbols_docs_and_notes() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::create_dir_all(src.path().join("docs/plans")).unwrap();
        fs::write(
            src.path().join("src/auth.rs"),
            "pub fn verify_token() {}\n",
        )
        .unwrap();
        fs::write(src.path().join("src/other.rs"), "pub fn misc() {}\n").unwrap();
        fs::write(
            src.path().join("docs/plans/harden-auth.md"),
            "# Harden authentication\n\nsrc/auth.rs.\n",
        )
        .unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = brain_store::Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();

        // A symbol name the path does not contain.
        let hits = find(&store, &index, "twin/app", "verify token").unwrap();
        assert_eq!(hits[0].label, "src/auth.rs", "{hits:?}");
        assert!(hits[0].why.iter().any(|w| w.contains("verify_token")), "{hits:?}");

        // Query hitting both the file and the plan.
        let hits = find(&store, &index, "twin/app", "auth").unwrap();
        let labels: Vec<&str> = hits.iter().map(|h| h.label.as_str()).collect();
        assert!(labels.contains(&"src/auth.rs"), "{hits:?}");
        assert!(labels.contains(&"harden-auth"), "{hits:?}");
        assert!(!labels.contains(&"src/other.rs"), "{hits:?}");

        let text = render(&hits, "auth", "twin/app", 10);
        assert!(text.contains("src/auth.rs"), "{text}");
    }
}
