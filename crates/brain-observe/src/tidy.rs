//! Tidy: the brain cleans up after its artifacts.
//!
//! An advisory scan first — every finding is one line with the fix named
//! — then `--fix` applies the *safe* set: re-renders from graph truth,
//! chmod re-arming, instruction-block regeneration, and governed moves to
//! the attic (each move is a `change` entity with intent and receipt —
//! auditable, revertible with `brain change revert`). Never automatic:
//! deletion of anything, edits to hand-written content, or moves of paths
//! with uncommitted git changes. A hand-edited projection is rescued into
//! the artifact's observation timeline before the re-render overwrites it
//! — agent work becomes history, not casualties.

use crate::kinds;
use crate::lifecycle;
use crate::projection::{self, DriftKind};
use crate::twin::{latest, live_from, live_to, observe_src, sid_label};
use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::{replay, Index, MemIndex};
use brain_store::{now_ms, Store, StoreError};
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

pub const ATTIC: &str = "docs/attic";

/// Whether a path already lives in the attic.
///
/// Every archival check needs this: a finding that proposes moving an
/// archived file deeper into the archive is a loop, not a cleanup.
pub fn archived(path: &str) -> bool {
    path == ATTIC || path.starts_with(&format!("{ATTIC}/"))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub category: String,
    pub path: String,
    pub detail: String,
    pub fix: String,
    /// Whether `--fix` may act on it (with `--cap fs` where it moves).
    pub fixable: bool,
}

/// The advisory scan. Deterministic order, one line per finding.
pub fn scan(
    store: &Store,
    index: &MemIndex,
    root: &Path,
    prefix: &str,
) -> Result<Vec<Finding>, StoreError> {
    let registry = kinds::registry(store, index)?;
    let mut out: Vec<Finding> = Vec::new();

    // Projection drift: hand edits, stale renders, missing files.
    for d in projection::drift(store, index, root, prefix)? {
        let (category, detail, fixable) = match d.kind {
            DriftKind::HandEdited => (
                "hand-edited-projection",
                "bytes differ from the last render; the edit will be rescued into the artifact's timeline, then re-rendered",
                true,
            ),
            DriftKind::StaleRender => {
                ("stale-render", "the graph moved on; re-render", true)
            }
            DriftKind::Missing => ("missing-projection", "projection file vanished; re-render", true),
        };
        out.push(Finding {
            category: category.to_string(),
            path: d.path,
            detail: detail.to_string(),
            fix: d.fix,
            fixable,
        });
    }

    // Read-only bit lost (metadata only — always safe to re-arm).
    for (name, node) in store.namespace()? {
        let Some(rel) = name.strip_prefix(&format!("{prefix}/")) else {
            continue;
        };
        let Ok(Object::Entity {
            id: sid,
            entity_kind,
            ..
        }) = store.get(&node)
        else {
            continue;
        };
        if entity_kind != "source_file" || latest(index, store, &sid, "expected_b3")?.is_none() {
            continue;
        }
        if let Ok(meta) = std::fs::metadata(root.join(rel)) {
            if !meta.permissions().readonly() {
                out.push(Finding {
                    category: "writable-projection".to_string(),
                    path: rel.to_string(),
                    detail: "read-only bit lost (clone/checkout)".to_string(),
                    fix: "re-armed automatically by tidy/refresh/hooks".to_string(),
                    fixable: true,
                });
            }
        }
    }

    // Artifact files whose artifact has left the present: archive.
    let doc_kinds = kinds::doc_kinds(store, index)?;
    for kind in &doc_kinds {
        let mut seen = BTreeSet::new();
        for node in index.entities_by_kind(kind) {
            let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
                continue;
            };
            if labels.get("prefix").map(String::as_str) != Some(prefix) || !seen.insert(id.clone())
            {
                continue;
            }
            let (state, why) = lifecycle::of(index, store, &id)?;
            let slug = labels.get("slug").cloned().unwrap_or_default();
            // Both the file an artifact is recorded in and the projection
            // rendered from it are archivable once the artifact concludes.
            let mut homes = live_from(index, store, &id, "recorded_in")?;
            homes.extend(live_from(index, store, &id, "projected_to")?);
            for (_, file) in homes {
                let path = sid_label(index, store, &file);
                if latest(index, store, &file, "present")?.as_deref() == Some("false")
                    || !std::path::Path::new(&root.join(&path)).exists()
                {
                    continue;
                }
                // Already archived. Without this, archiving is not
                // idempotent: the next run proposes moving the attic into
                // the attic, and the one after that moves that, forever.
                if archived(&path) {
                    continue;
                }
                if !state.is_active() {
                    out.push(Finding {
                        category: "retired-artifact-file".to_string(),
                        path: path.clone(),
                        detail: format!("{kind} '{slug}' is {} ({why})", state.as_str()),
                        fix: format!("governed move to {ATTIC}/{path}"),
                        fixable: true,
                    });
                    continue;
                }
                // Misplaced: active, but outside the kind's home globs.
                if let Some(def) = registry.get(kind.as_str()) {
                    if !def.home.is_empty()
                        && !def
                            .home
                            .iter()
                            .any(|g| crate::templates::glob_match(g, &path))
                        && latest(index, store, &file, "expected_b3")?.is_none()
                    {
                        let stem = path.rsplit('/').next().unwrap_or(&path);
                        let home_dir = def
                            .home
                            .first()
                            .map(|g| g.trim_end_matches("**").trim_end_matches("*.md"))
                            .unwrap_or("")
                            .trim_end_matches('/');
                        out.push(Finding {
                            category: "misplaced-artifact".to_string(),
                            path: path.clone(),
                            detail: format!(
                                "{kind} '{slug}' lives outside its home ({})",
                                def.home.join(", ")
                            ),
                            fix: format!("governed move to {home_dir}/{stem}"),
                            fixable: true,
                        });
                    }
                }
            }
        }
    }

    // Orphaned assets: owner concluded. Archive the bytes, keep the graph.
    for (path, why) in crate::assets::orphaned(store, index, prefix)? {
        if archived(&path) {
            continue;
        }
        out.push(Finding {
            category: "legacy-asset".to_string(),
            path: path.clone(),
            detail: why,
            fix: format!("governed move to {ATTIC}/{path} (entity and relations stay as history)"),
            fixable: true,
        });
    }

    // Concluded prototypes: archive the whole directory.
    let mut seen = BTreeSet::new();
    for node in index.entities_by_kind("prototype") {
        let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
            continue;
        };
        if labels.get("prefix").map(String::as_str) != Some(prefix) || !seen.insert(id.clone()) {
            continue;
        }
        let (state, _) = lifecycle::of(index, store, &id)?;
        if state.is_active() {
            continue;
        }
        for (_, file) in live_from(index, store, &id, "recorded_in")? {
            if latest(index, store, &file, "present")?.as_deref() == Some("false") {
                continue;
            }
            let readme = sid_label(index, store, &file);
            if let Some(dir) = readme.rsplit_once('/').map(|(d, _)| d.to_string()) {
                if archived(&dir) {
                    continue;
                }
                out.push(Finding {
                    category: "concluded-prototype".to_string(),
                    path: dir.clone(),
                    detail: format!(
                        "prototype '{}' is {}",
                        labels.get("slug").cloned().unwrap_or_default(),
                        state.as_str()
                    ),
                    fix: format!("governed move of the directory to {ATTIC}/{dir}"),
                    fixable: true,
                });
            }
        }
    }

    // Untyped documents: ingested markdown no kind claims.
    for (name, node) in store.namespace()? {
        let Some(rel) = name.strip_prefix(&format!("{prefix}/")) else {
            continue;
        };
        if !rel.ends_with(".md") {
            continue;
        }
        let Ok(Object::Entity {
            id: sid,
            entity_kind,
            ..
        }) = store.get(&node)
        else {
            continue;
        };
        if entity_kind != "source_file"
            || latest(index, store, &sid, "present")?.as_deref() == Some("false")
            || latest(index, store, &sid, "generated")?.as_deref() == Some("true")
            || !live_to(index, store, &sid, "recorded_in")?.is_empty()
        {
            continue;
        }
        let dir_glob = match rel.rsplit_once('/') {
            Some((d, _)) => format!("{d}/*.md"),
            None => "*.md".to_string(),
        };
        out.push(Finding {
            category: "untyped-document".to_string(),
            path: rel.to_string(),
            detail: "no artifact kind claims this document — it cannot rot visibly".to_string(),
            fix: format!(
                "teach it: brain template set <slug> --applies-to <kind> --capture \"{dir_glob}\" --fields \"title=heading\" --requires title"
            ),
            fixable: false,
        });
    }

    // Instruction blocks out of date with the registry.
    for file in crate::instructions::block_drift(store, index, root, prefix)? {
        out.push(Finding {
            category: "stale-instructions".to_string(),
            path: file,
            detail: "guardrail block differs from the kind registry".to_string(),
            fix: format!("brain instructions generate . --prefix {prefix}"),
            fixable: true,
        });
    }

    out.sort_by(|a, b| a.category.cmp(&b.category).then(a.path.cmp(&b.path)));
    out.dedup();
    Ok(out)
}

/// Explicit deletion — never chosen by the scan, only by a human/agent
/// naming the path. Intent before the effect, receipt after: even
/// deletions leave a trail.
pub fn remove_path(
    store: &Store,
    root: &Path,
    rel: &str,
    caps: &[String],
) -> Result<bool, StoreError> {
    if !caps.iter().any(|c| c == "fs") {
        return Err(StoreError::Io(std::io::Error::other(
            "refused: deletion requires --cap fs (no ambient authority)",
        )));
    }
    let now = now_ms();
    let intent = store.put(&Object::Intent {
        action: "fs/remove".to_string(),
        arg_hash: brain_core::canonical::hash_bytes(rel.as_bytes()),
        capability: Some("fs".to_string()),
        at_ms: now,
    })?;
    store.intents().begin(intent)?;
    let path = root.join(rel);
    let result = if path.is_dir() {
        std::fs::remove_dir_all(&path)
    } else {
        std::fs::remove_file(&path)
    };
    let ok = result.is_ok();
    let detail = match &result {
        Ok(()) => format!("fs/remove {rel}"),
        Err(e) => format!("fs/remove {rel} failed: {e}"),
    };
    let receipt = store.put(&Object::Receipt {
        intent,
        ok,
        detail,
        at_ms: now_ms(),
    })?;
    if ok {
        store.intents().confirm(intent, receipt)?;
    } else {
        store.intents().fail(intent, receipt)?;
    }
    Ok(ok)
}

fn git_dirty(root: &Path, rel: &str) -> bool {
    Command::new("git")
        .args(["-C"])
        .arg(root)
        .args(["status", "--porcelain", "--"])
        .arg(rel)
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false)
}

/// Apply the safe fixes. Content-touching fixes and moves require the
/// `fs` capability; metadata fixes (chmod) apply regardless. Returns
/// (fixed, skipped-with-reason).
pub fn fix(
    store: &Store,
    root: &Path,
    prefix: &str,
    findings: &[Finding],
    caps: &[String],
) -> Result<(Vec<String>, Vec<(String, String)>), StoreError> {
    let has_fs = caps.iter().any(|c| c == "fs");
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    let registry = kinds::registry(store, &index)?;
    let mut fixed = Vec::new();
    let mut skipped = Vec::new();
    let now = now_ms();

    for f in findings {
        match f.category.as_str() {
            "writable-projection" => {
                projection::reapply_readonly(store, &index, root, prefix)?;
                fixed.push(format!("{} re-armed read-only", f.path));
            }
            "stale-instructions" => {
                if !has_fs {
                    skipped.push((f.path.clone(), "needs --cap fs".to_string()));
                    continue;
                }
                crate::instructions::generate(store, &index, root, prefix)?;
                fixed.push(format!("{} regenerated", f.path));
            }
            "hand-edited-projection" | "stale-render" | "missing-projection" => {
                if !has_fs {
                    skipped.push((f.path.clone(), "needs --cap fs".to_string()));
                    continue;
                }
                // Identify the projecting artifact and its kind.
                let file_sid = StableId::derive(&["file", &f.path]);
                let Some((_, artifact)) = live_to(&index, store, &file_sid, "projected_to")?
                    .into_iter()
                    .next()
                else {
                    skipped.push((f.path.clone(), "no projecting artifact".to_string()));
                    continue;
                };
                let mut kind_slug = None;
                for anode in index.entity_nodes(&artifact) {
                    if let Ok(Object::Entity {
                        entity_kind,
                        labels,
                        ..
                    }) = store.get(&anode)
                    {
                        kind_slug =
                            Some((entity_kind, labels.get("slug").cloned().unwrap_or_default()));
                        break;
                    }
                }
                let Some((kind, slug)) = kind_slug else {
                    skipped.push((f.path.clone(), "artifact unreadable".to_string()));
                    continue;
                };
                // Rescue the hand-edit into the timeline first.
                if f.category == "hand-edited-projection" {
                    if let Ok(bytes) = std::fs::read(root.join(&f.path)) {
                        let text = String::from_utf8_lossy(&bytes);
                        observe_src(store, &artifact, "hand_edit", &text, "tidy", now)?;
                        observe_src(
                            store,
                            &artifact,
                            "note",
                            &format!(
                                "hand-edit rescued from {} before re-render; content preserved in the hand_edit timeline",
                                f.path
                            ),
                            "tidy",
                            now,
                        )?;
                    }
                }
                let Some(def) = registry.get(&kind) else {
                    skipped.push((f.path.clone(), format!("unknown kind {kind}")));
                    continue;
                };
                let Some(content) = latest(&index, store, &artifact, "content")? else {
                    skipped.push((f.path.clone(), "artifact has no content".to_string()));
                    continue;
                };
                let Some(rel) = projection::projection_rel(def, &slug) else {
                    skipped.push((f.path.clone(), "kind has no projection path".to_string()));
                    continue;
                };
                let body = projection::render_body(&rel, &kind, prefix, &slug, &content);
                projection::write_projection(store, &index, root, &artifact, &rel, &body)?;
                fixed.push(format!("{} re-rendered from the graph", f.path));
            }
            "retired-artifact-file"
            | "legacy-asset"
            | "misplaced-artifact"
            | "concluded-prototype" => {
                if !has_fs {
                    skipped.push((f.path.clone(), "needs --cap fs".to_string()));
                    continue;
                }
                if git_dirty(root, &f.path) {
                    skipped.push((
                        f.path.clone(),
                        "uncommitted git changes — commit or stash first".to_string(),
                    ));
                    continue;
                }
                let dest = if f.category == "misplaced-artifact" {
                    // fix text: "governed move to <dest>"
                    f.fix.trim_start_matches("governed move to ").to_string()
                } else {
                    format!("{ATTIC}/{}", f.path)
                };
                let p = crate::govern::propose_move(
                    store,
                    root,
                    prefix,
                    &f.path,
                    &dest,
                    &format!("tidy: {}", f.category),
                )?;
                let applied = crate::govern::apply(store, root, prefix, &p.slug, caps)?;
                if applied.ok {
                    fixed.push(format!(
                        "{} -> {dest} (change '{}', revertible)",
                        f.path, p.slug
                    ));
                } else {
                    skipped.push((
                        f.path.clone(),
                        "move failed — see the change receipt".into(),
                    ));
                }
            }
            _ => skipped.push((f.path.clone(), "advisory only".to_string())),
        }
    }
    Ok((fixed, skipped))
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
    fn tidy_finds_and_fixes_rescuing_hand_edits() {
        let src = tempfile::tempdir().unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        crate::templates::seed(&store).unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::write(src.path().join("src/main.rs"), "pub fn main() {}\n").unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        // A graph-first plan with its projection.
        let out = crate::twin::author_artifact(
            &store,
            "twin/app",
            "plan",
            "sprint",
            "Sprint",
            "# Sprint\n\nShip it.\n",
            "agent",
        )
        .unwrap();
        let index = fresh_index(&store);
        let rel = "docs/brain/plans/sprint.md";
        let body =
            projection::render_body(rel, "plan", "twin/app", "sprint", "# Sprint\n\nShip it.\n");
        projection::write_projection(&store, &index, src.path(), &out.sid, rel, &body).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        // Clean scan (the plan is healthy).
        let index = fresh_index(&store);
        let findings = scan(&store, &index, src.path(), "twin/app").unwrap();
        assert!(
            findings.iter().all(|f| f.category == "untyped-document"),
            "only advisory leftovers: {findings:?}"
        );

        // Defeat the bit, hand-edit the projection.
        let target = src.path().join(rel);
        let mut perms = fs::metadata(&target).unwrap().permissions();
        perms.set_readonly(false);
        fs::set_permissions(&target, perms).unwrap();
        fs::write(&target, "my sneaky rewrite\n").unwrap();
        let index = fresh_index(&store);
        let findings = scan(&store, &index, src.path(), "twin/app").unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.category == "hand-edited-projection" && f.path == rel),
            "{findings:?}"
        );

        // Fix: rescue + re-render, read-only again.
        let (fixed, _skipped) = fix(
            &store,
            src.path(),
            "twin/app",
            &findings,
            &["fs".to_string()],
        )
        .unwrap();
        assert!(fixed.iter().any(|m| m.contains("re-rendered")), "{fixed:?}");
        assert_eq!(fs::read_to_string(&target).unwrap(), body, "graph won");
        assert!(fs::metadata(&target).unwrap().permissions().readonly());
        let index = fresh_index(&store);
        let rescued = latest(&index, &store, &out.sid, "hand_edit")
            .unwrap()
            .unwrap();
        assert!(
            rescued.contains("sneaky"),
            "the edit is history, not a casualty"
        );

        // Conclude the plan: its projection file becomes archivable.
        crate::lifecycle::set(
            &store,
            &index,
            &out.sid,
            crate::lifecycle::Lifecycle::Done,
            None,
        )
        .unwrap();
        let index = fresh_index(&store);
        let findings = scan(&store, &index, src.path(), "twin/app").unwrap();
        let retired: Vec<_> = findings
            .iter()
            .filter(|f| f.category == "retired-artifact-file")
            .collect();
        assert_eq!(retired.len(), 1, "{findings:?}");
        // Not a git repo -> moves are allowed (nothing is dirty).
        let (fixed, skipped) = fix(
            &store,
            src.path(),
            "twin/app",
            &findings,
            &["fs".to_string()],
        )
        .unwrap();
        assert!(
            fixed.iter().any(|m| m.contains("docs/attic/")),
            "fixed={fixed:?} skipped={skipped:?}"
        );
        assert!(src.path().join(format!("{ATTIC}/{rel}")).exists());
        assert!(!target.exists());

        // Archiving is a fixed point. Without the attic guard the next
        // scan proposes moving docs/attic into docs/attic/docs/attic, and
        // cleanup becomes a treadmill.
        crate::twin::refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);
        let again = scan(&store, &index, src.path(), "twin/app").unwrap();
        assert!(
            !again.iter().any(|f| f.path.starts_with(ATTIC)),
            "tidy wants to archive the archive: {again:?}"
        );

        // The move is a governed change: auditable and revertible.
        let index = fresh_index(&store);
        let mut change_found = false;
        for node in index.entities_by_kind("change") {
            if let Ok(Object::Entity { id, labels, .. }) = store.get(&node) {
                if labels.get("target").map(String::as_str) == Some(rel) {
                    assert_eq!(
                        latest(&index, &store, &id, "status").unwrap().as_deref(),
                        Some("applied")
                    );
                    change_found = true;
                }
            }
        }
        assert!(change_found, "tidy's move left a change trail");

        // Without the capability, nothing content-touching happens.
        let findings = scan(&store, &fresh_index(&store), src.path(), "twin/app").unwrap();
        let (_, skipped) = fix(&store, src.path(), "twin/app", &findings, &[]).unwrap();
        assert!(skipped
            .iter()
            .all(|(_, why)| why.contains("cap fs") || why.contains("advisory")));
    }
}
