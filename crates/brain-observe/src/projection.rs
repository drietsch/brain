//! The read-only projection contract: files rendered from the graph can
//! be read by anything and edited by nothing.
//!
//! Three layers keep graph-first artifacts from drifting back into
//! hand-edited files (ADR-019):
//!
//! 1. **Marker** — every projection's first line names the authoring
//!    command, because that line is what an agent reads first.
//! 2. **Filesystem** — rendered files carry mode 0444. Best-effort (git
//!    does not preserve the bit), re-armed by refresh, hooks, and tidy.
//! 3. **Detection** — the authoritative layer: `expected_b3` on the file
//!    entity records the exact bytes rendered; any mismatch on disk is a
//!    reported violation with the fix spelled out, never a silent state.
//!
//! Repair re-renders from the graph — the graph always wins — but tidy
//! rescues the hand-edit into the artifact's observation timeline first;
//! agent work is preserved as history, not destroyed.

use crate::kinds::KindDef;
use crate::twin::{latest, observe_src, relate};
use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::{Index, MemIndex};
use brain_store::{now_ms, Store, StoreError};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The relative path a kind's projection renders to.
pub fn projection_rel(def: &KindDef, slug: &str) -> Option<String> {
    if def.project_to.is_empty() {
        return None;
    }
    Some(def.project_to.replace("{slug}", slug))
}

/// The first-line marker: identifies the file as generated, read-only,
/// and names the command that edits the artifact behind it.
pub fn marker(rel_path: &str, kind: &str, prefix: &str, slug: &str) -> String {
    let edit = format!("brain artifact edit {prefix} {kind} {slug} --file <md>");
    if rel_path.ends_with(".1") {
        format!(".\\\" brain:projection kind={kind} slug={slug} — GENERATED, READ-ONLY. Edit via: {edit}\n")
    } else if rel_path.ends_with(".md") {
        format!("<!-- brain:projection kind={kind} slug={slug} — GENERATED, READ-ONLY. Edit via: {edit} -->\n")
    } else {
        format!("# brain:projection kind={kind} slug={slug} — GENERATED, READ-ONLY. Edit via: {edit}\n")
    }
}

/// The full projection body: marker + content. Deterministic — two
/// renders of the same artifact are byte-identical.
pub fn render_body(rel_path: &str, kind: &str, prefix: &str, slug: &str, content: &str) -> String {
    let mut body = marker(rel_path, kind, prefix, slug);
    body.push('\n');
    body.push_str(content);
    if !content.ends_with('\n') {
        body.push('\n');
    }
    body
}

fn set_readonly(path: &Path, readonly: bool) {
    if let Ok(meta) = fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_readonly(readonly);
        let _ = fs::set_permissions(path, perms);
    }
}

/// Atomically write a projection file, arm the read-only bit, and record
/// the contract in the graph: `generated=true` + `expected_b3` on the
/// file entity, `projected_to` from the artifact. Guarded — re-rendering
/// unchanged bytes writes no objects (the file itself is rewritten only
/// when its bytes differ).
pub fn write_projection(
    store: &Store,
    index: &MemIndex,
    root: &Path,
    artifact: &StableId,
    rel_path: &str,
    body: &str,
) -> Result<PathBuf, StoreError> {
    let target = root.join(rel_path);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let unchanged = fs::read(&target).map(|cur| cur == body.as_bytes()).unwrap_or(false);
    if !unchanged {
        set_readonly(&target, false);
        let tmp = target.with_extension("tmp");
        fs::write(&tmp, body.as_bytes())?;
        fs::rename(&tmp, &target)?;
    }
    set_readonly(&target, true);

    let file_sid = StableId::derive(&["file", rel_path]);
    let mut labels = BTreeMap::new();
    labels.insert("path".to_string(), rel_path.to_string());
    store.put(&Object::Entity {
        id: file_sid.clone(),
        entity_kind: "source_file".to_string(),
        labels,
    })?;
    let now = now_ms();
    let hash = blake3::hash(body.as_bytes()).to_hex().to_string();
    if latest(index, store, &file_sid, "generated")?.as_deref() != Some("true") {
        observe_src(store, &file_sid, "generated", "true", "projection", now)?;
    }
    if latest(index, store, &file_sid, "expected_b3")?.as_deref() != Some(hash.as_str()) {
        observe_src(store, &file_sid, "expected_b3", &hash, "projection", now)?;
    }
    let mut written: BTreeSet<(StableId, String, StableId)> = BTreeSet::new();
    relate(store, index, &mut written, artifact, "projected_to", &file_sid, now)?;
    Ok(target)
}

/// Re-arm the read-only bit on every projection under a prefix (git does
/// not preserve it across clones and checkouts). Returns files touched.
pub fn reapply_readonly(
    store: &Store,
    index: &MemIndex,
    root: &Path,
    prefix: &str,
) -> Result<usize, StoreError> {
    let mut armed = 0;
    for (name, node) in store.namespace()? {
        let Some(rel) = name.strip_prefix(&format!("{prefix}/")) else { continue };
        let Ok(Object::Entity { id: sid, entity_kind, .. }) = store.get(&node) else { continue };
        if entity_kind != "source_file"
            || latest(index, store, &sid, "expected_b3")?.is_none()
        {
            continue;
        }
        let path = root.join(rel);
        if let Ok(meta) = fs::metadata(&path) {
            if !meta.permissions().readonly() {
                set_readonly(&path, true);
                armed += 1;
            }
        }
    }
    Ok(armed)
}

#[derive(Debug, Clone, PartialEq)]
pub enum DriftKind {
    /// Bytes on disk differ from the last render: someone edited the file.
    HandEdited,
    /// The projection file vanished.
    Missing,
    /// The graph moved on: rendering the artifact now would produce
    /// different bytes than the file carries.
    StaleRender,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Drift {
    pub path: String,
    pub kind: DriftKind,
    /// The command that resolves it — agents obey actionable output.
    pub fix: String,
}

/// The authoritative detection layer: compare every projection's disk
/// bytes against its `expected_b3`, and its expected bytes against what a
/// fresh render would produce. Query-time, never stored.
pub fn drift(
    store: &Store,
    index: &MemIndex,
    root: &Path,
    prefix: &str,
) -> Result<Vec<Drift>, StoreError> {
    let registry = crate::kinds::registry(store, index)?;
    let mut out = Vec::new();
    for (name, node) in store.namespace()? {
        let Some(rel) = name.strip_prefix(&format!("{prefix}/")) else { continue };
        let Ok(Object::Entity { id: sid, entity_kind, .. }) = store.get(&node) else { continue };
        if entity_kind != "source_file" {
            continue;
        }
        let Some(expected) = latest(index, store, &sid, "expected_b3")? else { continue };

        // Who projects here, and of what kind?
        let mut source: Option<(StableId, String, String)> = None; // artifact, kind, slug
        for (_, artifact) in crate::twin::live_to(index, store, &sid, "projected_to")? {
            for anode in index.entity_nodes(&artifact) {
                if let Ok(Object::Entity { entity_kind: ak, labels: al, .. }) = store.get(&anode)
                {
                    source = Some((
                        artifact.clone(),
                        ak,
                        al.get("slug").cloned().unwrap_or_default(),
                    ));
                    break;
                }
            }
        }

        let fix = match &source {
            Some((_, kind, slug)) => {
                format!("brain artifact edit {prefix} {kind} {slug} --file <md>  (or restore: brain artifact render . --prefix {prefix})")
            }
            None => format!("brain docs generate . --prefix {prefix}"),
        };

        match fs::read(root.join(rel)) {
            Err(_) => {
                // A deleted projection of a retired artifact is tidy's
                // business, not drift; only flag when the file entity
                // still claims presence.
                if latest(index, store, &sid, "present")?.as_deref() != Some("false") {
                    out.push(Drift {
                        path: rel.to_string(),
                        kind: DriftKind::Missing,
                        fix: fix.clone(),
                    });
                }
            }
            Ok(bytes) => {
                let on_disk = blake3::hash(&bytes).to_hex().to_string();
                if on_disk != expected {
                    out.push(Drift {
                        path: rel.to_string(),
                        kind: DriftKind::HandEdited,
                        fix: format!(
                            "the graph is the source of truth — {fix}; a hand-edit here is rescued then re-rendered by `brain tidy . --prefix {prefix} --fix --cap fs`"
                        ),
                    });
                    continue;
                }
                // Stale render: the artifact's current content no longer
                // matches what was rendered.
                if let Some((artifact, kind, slug)) = &source {
                    if let (Some(def), Some(content)) = (
                        registry.get(kind),
                        latest(index, store, artifact, "content")?,
                    ) {
                        if let Some(rel_expected) = projection_rel(def, slug) {
                            if rel_expected == rel {
                                let fresh =
                                    render_body(rel, kind, prefix, slug, &content);
                                if blake3::hash(fresh.as_bytes()).to_hex().to_string() != expected
                                {
                                    out.push(Drift {
                                        path: rel.to_string(),
                                        kind: DriftKind::StaleRender,
                                        fix: format!(
                                            "brain artifact render . --prefix {prefix} --kind {kind}"
                                        ),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain_index::replay;

    fn fresh_index(store: &Store) -> MemIndex {
        let mut index = MemIndex::new();
        replay(store, &mut index).unwrap();
        index
    }

    #[test]
    fn projections_render_readonly_and_drift_is_detected() {
        let root = tempfile::tempdir().unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        let artifact = StableId::derive(&["plan", "twin/app", "big-plan"]);
        store
            .put(&Object::Entity {
                id: artifact.clone(),
                entity_kind: "plan".to_string(),
                labels: {
                    let mut l = BTreeMap::new();
                    l.insert("prefix".to_string(), "twin/app".to_string());
                    l.insert("slug".to_string(), "big-plan".to_string());
                    l
                },
            })
            .unwrap();
        observe_src(&store, &artifact, "content", "# Big Plan\n\nDo things.\n", "agent", 10)
            .unwrap();

        let index = fresh_index(&store);
        let rel = "docs/brain/plans/big-plan.md";
        let body = render_body(rel, "plan", "twin/app", "big-plan", "# Big Plan\n\nDo things.\n");
        assert_eq!(body, render_body(rel, "plan", "twin/app", "big-plan", "# Big Plan\n\nDo things.\n"), "deterministic");
        let target =
            write_projection(&store, &index, root.path(), &artifact, rel, &body).unwrap();
        assert!(target.exists());
        assert!(fs::metadata(&target).unwrap().permissions().readonly(), "chmod armed");
        let text = fs::read_to_string(&target).unwrap();
        assert!(text.starts_with("<!-- brain:projection kind=plan"), "{text}");
        assert!(text.contains("brain artifact edit twin/app plan big-plan"), "marker names the fix");

        // Bind the file so drift/reapply can find it under the prefix.
        let file_sid = StableId::derive(&["file", rel]);
        let node = store
            .put(&Object::Entity {
                id: file_sid.clone(),
                entity_kind: "source_file".to_string(),
                labels: {
                    let mut l = BTreeMap::new();
                    l.insert("path".to_string(), rel.to_string());
                    l
                },
            })
            .unwrap();
        store.bind(&format!("twin/app/{rel}"), node).unwrap();

        // Idempotent re-render: no objects, no drift.
        let index = fresh_index(&store);
        let before = store.count_objects().unwrap();
        write_projection(&store, &index, root.path(), &artifact, rel, &body).unwrap();
        assert_eq!(store.count_objects().unwrap(), before);
        assert!(drift(&store, &index, root.path(), "twin/app").unwrap().is_empty());

        // A hand-edit is detected even after the read-only bit is defeated.
        set_readonly(&target, false);
        fs::write(&target, "sneaky edit\n").unwrap();
        let found = drift(&store, &index, root.path(), "twin/app").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, DriftKind::HandEdited);
        assert!(found[0].fix.contains("brain artifact edit"), "{}", found[0].fix);

        // Repair by re-render; reapply_readonly re-arms lost bits.
        write_projection(&store, &index, root.path(), &artifact, rel, &body).unwrap();
        assert!(drift(&store, &index, root.path(), "twin/app").unwrap().is_empty());
        set_readonly(&target, false);
        let armed = reapply_readonly(&store, &index, root.path(), "twin/app").unwrap();
        assert_eq!(armed, 1);
        assert!(fs::metadata(&target).unwrap().permissions().readonly());

        // The graph moves on: same file, newer artifact content -> stale render.
        observe_src(&store, &artifact, "content", "# Big Plan\n\nDo MORE things.\n", "agent", 20)
            .unwrap();
        let index = fresh_index(&store);
        let found = drift(&store, &index, root.path(), "twin/app").unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].kind, DriftKind::StaleRender);

        // Missing projection is flagged.
        set_readonly(&target, false);
        fs::remove_file(&target).unwrap();
        let found = drift(&store, &index, root.path(), "twin/app").unwrap();
        assert!(found.iter().any(|d| d.kind == DriftKind::Missing));
    }
}
