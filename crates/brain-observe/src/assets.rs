//! Assets: typed binary artifacts — bytes in files, meaning in the graph.
//!
//! A screenshot, an HTML template, a diagram: the twin already hashes the
//! bytes (media extensions are ingestible), but an anonymous hashed file
//! can neither rot visibly nor be tidied when its purpose ends. An asset
//! entity adds what the bytes cannot carry: a subtype, an owner
//! (`attached_to` — the plan/feature/template it belongs to), and declared
//! `depicts` targets — media cannot be substring-scanned for mentions, so
//! the links are stated at capture time. Staleness then reuses the
//! ordinary machinery: a depicted target that changed after the asset's
//! bytes were captured makes the asset stale; a retired owner makes it
//! tidy-able. Bytes never enter graph objects (canonical JSON, no blobs).

use crate::twin::{latest, observe_src, relate, sid_label};
use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::{replay, Index, MemIndex};
use brain_store::{now_ms, Store, StoreError};
use std::collections::{BTreeMap, BTreeSet};

pub fn asset_sid(prefix: &str, slug: &str) -> StableId {
    StableId::derive(&["asset", prefix, slug])
}

/// Subtype inferred from the file extension when not declared.
pub fn infer_subtype(rel_path: &str) -> &'static str {
    match rel_path.rsplit('.').next().unwrap_or("") {
        "png" | "svg" | "gif" => "image",
        "webm" | "mp4" => "screencast",
        "wav" => "audio",
        "html" => "template",
        _ => "file",
    }
}

#[derive(Debug)]
pub struct AssetOutcome {
    pub sid: StableId,
    pub slug: String,
    pub wrote: bool,
}

/// Declare an asset: type its twinned file, attach it to its owner, and
/// state what it depicts. All writes guarded; re-declaring is a no-op.
pub fn add(
    store: &Store,
    prefix: &str,
    rel_path: &str,
    owner: &StableId,
    depicts: &[StableId],
    subtype: Option<&str>,
) -> Result<AssetOutcome, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    let mut written = BTreeSet::new();
    declare(
        store,
        &index,
        &mut written,
        prefix,
        rel_path,
        owner,
        depicts,
        subtype,
        now_ms(),
    )
}

/// `add` against an index the caller already holds.
///
/// Ingest paths that declare many assets in one pass — a Playwright run
/// with a screenshot per failure, the docs pipeline with a screenshot per
/// section — must not replay the whole log once per file.
#[allow(clippy::too_many_arguments)]
pub fn declare(
    store: &Store,
    index: &MemIndex,
    written: &mut BTreeSet<(StableId, String, StableId)>,
    prefix: &str,
    rel_path: &str,
    owner: &StableId,
    depicts: &[StableId],
    subtype: Option<&str>,
    now: u64,
) -> Result<AssetOutcome, StoreError> {
    let file_sid = StableId::derive(&["file", rel_path]);
    let slug = crate::docs::slug_of(rel_path);
    let sid = asset_sid(prefix, &slug);

    let mut labels = BTreeMap::new();
    labels.insert("prefix".to_string(), prefix.to_string());
    labels.insert("slug".to_string(), slug.clone());
    labels.insert("path".to_string(), rel_path.to_string());
    store.put(&Object::Entity {
        id: sid.clone(),
        entity_kind: "asset".to_string(),
        labels,
    })?;

    let mut wrote = false;
    let subtype = subtype.unwrap_or_else(|| infer_subtype(rel_path));
    if latest(index, store, &sid, "subtype")?.as_deref() != Some(subtype) {
        observe_src(store, &sid, "subtype", subtype, "agent", now)?;
        wrote = true;
    }

    let repo_sid = StableId::derive(&["repo", prefix]);
    for (kind, to) in [
        ("recorded_in", &file_sid),
        ("attached_to", owner),
        ("concerns", &repo_sid),
    ] {
        if relate(store, index, written, &sid, kind, to, now)? {
            wrote = true;
        }
    }
    for target in depicts {
        if relate(store, index, written, &sid, "depicts", target, now)? {
            wrote = true;
        }
    }
    Ok(AssetOutcome { sid, slug, wrote })
}

#[derive(Debug)]
pub struct AssetRow {
    pub slug: String,
    pub path: String,
    pub subtype: String,
    pub owner: Option<String>,
    pub lifecycle: crate::lifecycle::Lifecycle,
}

/// All assets under a prefix, with their owner labels and lifecycle.
pub fn list(store: &Store, index: &MemIndex, prefix: &str) -> Result<Vec<AssetRow>, StoreError> {
    let mut seen: BTreeSet<StableId> = BTreeSet::new();
    let mut out = Vec::new();
    for node in index.entities_by_kind("asset") {
        let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
            continue;
        };
        if labels.get("prefix").map(String::as_str) != Some(prefix) || !seen.insert(id.clone()) {
            continue;
        }
        // An owner with no display label (a repository) would otherwise
        // print its raw identifier, which tells a reader nothing.
        let owner = crate::twin::live_from(index, store, &id, "attached_to")?
            .first()
            .map(|(_, o)| {
                let label = sid_label(index, store, o);
                if !label.starts_with("sid:") {
                    return label;
                }
                for node in index.entity_nodes(o) {
                    if let Ok(Object::Entity { labels, .. }) = store.get(&node) {
                        if let Some(prefix) = labels.get("prefix") {
                            return prefix.clone();
                        }
                    }
                }
                label
            });
        out.push(AssetRow {
            slug: labels.get("slug").cloned().unwrap_or_default(),
            path: labels.get("path").cloned().unwrap_or_default(),
            subtype: latest(index, store, &id, "subtype")?.unwrap_or_else(|| "file".into()),
            owner,
            lifecycle: crate::lifecycle::of(index, store, &id)?.0,
        });
    }
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    Ok(out)
}

/// Stale assets: active, with a live `depicts` target whose content
/// changed after the asset's bytes were last captured (or acknowledged).
/// Returned in the same shape as document staleness so every surface
/// (stale, attention, wake) treats them uniformly.
pub fn stale(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
) -> Result<Vec<(String, Vec<String>)>, StoreError> {
    let mut out = Vec::new();
    let mut seen: BTreeSet<StableId> = BTreeSet::new();
    for node in index.entities_by_kind("asset") {
        let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
            continue;
        };
        if labels.get("prefix").map(String::as_str) != Some(prefix) || !seen.insert(id.clone()) {
            continue;
        }
        if !crate::lifecycle::of(index, store, &id)?.0.is_active() {
            continue;
        }
        // The asset's effective time: its file's newest bytes, or a later ack.
        let mut captured_at = 0u64;
        for (_, file) in crate::twin::live_from(index, store, &id, "recorded_in")? {
            if let Some((at, _)) = crate::twin::latest_at(index, store, &file, "content_b3")? {
                captured_at = captured_at.max(at);
            }
        }
        if let Some((ack_at, _)) = crate::twin::latest_at(index, store, &id, "reviewed")? {
            captured_at = captured_at.max(ack_at);
        }
        if captured_at == 0 {
            continue;
        }
        let mut changed = Vec::new();
        for (_, target) in crate::twin::live_from(index, store, &id, "depicts")? {
            if let Some((at, _)) = crate::twin::latest_at(index, store, &target, "content_b3")? {
                if at > captured_at {
                    changed.push(sid_label(index, store, &target));
                }
            }
        }
        if !changed.is_empty() {
            changed.sort();
            out.push((labels.get("slug").cloned().unwrap_or_default(), changed));
        }
    }
    out.sort();
    Ok(out)
}

/// Resolve a `--depicts` argument: a twinned file path or any kind/slug.
pub fn resolve_depicts(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    name: &str,
) -> Result<Option<StableId>, StoreError> {
    Ok(crate::features::resolve_target(store, index, prefix, name)?.map(|(sid, _)| sid))
}

/// A dangling asset: its owner is no longer active. Tidy's signal.
pub fn orphaned(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
) -> Result<Vec<(String, String)>, StoreError> {
    let mut out = Vec::new();
    let mut seen: BTreeSet<StableId> = BTreeSet::new();
    for node in index.entities_by_kind("asset") {
        let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
            continue;
        };
        if labels.get("prefix").map(String::as_str) != Some(prefix) || !seen.insert(id.clone()) {
            continue;
        }
        if !crate::lifecycle::of(index, store, &id)?.0.is_active() {
            continue;
        }
        for (_, owner) in crate::twin::live_from(index, store, &id, "attached_to")? {
            let (state, _) = crate::lifecycle::of(index, store, &owner)?;
            if !state.is_active() {
                out.push((
                    labels.get("path").cloned().unwrap_or_default(),
                    format!(
                        "owner {} is {}",
                        sid_label(index, store, &owner),
                        state.as_str()
                    ),
                ));
            }
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::twin::refresh;
    use std::fs;

    fn fresh_index(store: &Store) -> MemIndex {
        let mut index = MemIndex::new();
        replay(store, &mut index).unwrap();
        index
    }

    #[test]
    fn assets_carry_ownership_and_rot_when_depicted_targets_change() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("docs/assets")).unwrap();
        fs::create_dir_all(src.path().join("docs/plans")).unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::write(src.path().join("docs/assets/flow.svg"), "<svg>1</svg>").unwrap();
        fs::write(
            src.path().join("docs/plans/build.md"),
            "# Build\n\nsrc/ui.rs.\n",
        )
        .unwrap();
        fs::write(src.path().join("src/ui.rs"), "pub fn ui() {}\n").unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let plan = StableId::derive(&["plan", "twin/app", "build"]);
        let ui = StableId::derive(&["file", "src/ui.rs"]);
        let out = add(
            &store,
            "twin/app",
            "docs/assets/flow.svg",
            &plan,
            &[ui.clone()],
            None,
        )
        .unwrap();
        assert!(out.wrote);
        // Re-declaring writes nothing.
        let before = store.count_objects().unwrap();
        let again = add(
            &store,
            "twin/app",
            "docs/assets/flow.svg",
            &plan,
            &[ui.clone()],
            None,
        )
        .unwrap();
        assert!(!again.wrote);
        assert_eq!(store.count_objects().unwrap(), before);

        let index = fresh_index(&store);
        let rows = list(&store, &index, "twin/app").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].subtype, "image");
        assert_eq!(rows[0].owner.as_deref(), Some("docs/plans/build.md"));
        assert!(stale(&store, &index, "twin/app").unwrap().is_empty());

        // The depicted file changes: the asset rots visibly.
        std::thread::sleep(std::time::Duration::from_millis(3));
        fs::write(src.path().join("src/ui.rs"), "pub fn ui() { /* v2 */ }\n").unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);
        let rotten = stale(&store, &index, "twin/app").unwrap();
        assert_eq!(rotten.len(), 1, "{rotten:?}");
        assert_eq!(rotten[0].0, "flow");
        assert_eq!(rotten[0].1, vec!["src/ui.rs".to_string()]);

        // Re-capturing the bytes (or acking) clears it.
        std::thread::sleep(std::time::Duration::from_millis(3));
        fs::write(src.path().join("docs/assets/flow.svg"), "<svg>2</svg>").unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);
        assert!(stale(&store, &index, "twin/app").unwrap().is_empty());

        // Concluding the owner orphans the asset for tidy.
        crate::lifecycle::set(
            &store,
            &index,
            &plan,
            crate::lifecycle::Lifecycle::Done,
            None,
        )
        .unwrap();
        let index = fresh_index(&store);
        let orphans = orphaned(&store, &index, "twin/app").unwrap();
        assert_eq!(orphans.len(), 1);
        assert!(orphans[0].1.contains("done"), "{orphans:?}");
    }
}
