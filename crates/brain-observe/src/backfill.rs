//! Backfill: replay git history into the twin, so a brownfield repo
//! arrives with its past already in the graph.
//!
//! Every historical fact is written with its **commit's timestamp**, not
//! now — so backfilled observations slot beneath current state in every
//! timeline: churn counts real history, `brain twin at <old-commit>`
//! works, and association's co-change signal gains every commit ever made
//! (files in one commit share one timestamp — a co-change batch).
//!
//! Deliberate limits: only file-level facts are backfilled (content
//! hashes, presence, per-commit repo state). Historical *structure*
//! (symbols, imports) is not reconstructed — the cost is enormous and the
//! current refresh covers the present. Facts are sourced `"backfill"`.
//!
//! Idempotent by construction: identical historical facts hash to
//! identical objects, so a re-run writes nothing.

use crate::{INGEST_EXTENSIONS, INGEST_FILENAMES};
use brain_core::ids::{NodeId, StableId};
use brain_core::object::Object;
use brain_store::{Store, StoreError};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

/// Blobs beyond this size are hashed as "skipped" rather than read —
/// backfill is orientation, not an archive.
const MAX_BLOB: usize = 4 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct BackfillReport {
    pub commits: usize,
    pub file_versions: usize,
    pub deletions: usize,
    pub skipped_blobs: usize,
    /// Objects actually written (re-runs approach zero).
    pub objects_written: usize,
}

struct FileEvent {
    status: char,
    path: String,
    renamed_to: Option<String>,
}

struct CommitEntry {
    hash: String,
    at_ms: u64,
    events: Vec<FileEvent>,
}

/// Replay up to `max_commits` of history (oldest first; 0 = all) under
/// `prefix`.
pub fn backfill(
    store: &Store,
    root: &Path,
    prefix: &str,
    max_commits: usize,
) -> Result<BackfillReport, StoreError> {
    let commits = read_history(root)?;
    let commits: Vec<&CommitEntry> = if max_commits > 0 && commits.len() > max_commits {
        commits[commits.len() - max_commits..].iter().collect()
    } else {
        commits.iter().collect()
    };

    let mut report = BackfillReport::default();
    let before = store.count_objects()?;
    let ns = store.namespace()?;
    let mut bindings: Vec<(String, NodeId)> = Vec::new();
    let mut known_entities: BTreeSet<String> = BTreeSet::new();
    let mut deleted: BTreeSet<String> = BTreeSet::new();

    let repo_sid = StableId::derive(&["repo", prefix]);
    let mut labels = BTreeMap::new();
    labels.insert("prefix".to_string(), prefix.to_string());
    let repo_node = store.put(&Object::Entity {
        id: repo_sid.clone(),
        entity_kind: "repo".to_string(),
        labels,
    })?;
    if !ns.contains_key(prefix) {
        bindings.push((prefix.to_string(), repo_node));
    }

    for commit in &commits {
        report.commits += 1;
        // The repo's state marker at this moment: powers `twin at <hash>`.
        observe(store, &repo_sid, "git_commit", &commit.hash, commit.at_ms)?;

        for ev in &commit.events {
            match ev.status {
                'D' => {
                    if !ingestible(&ev.path) {
                        continue;
                    }
                    ensure_entity(store, prefix, &ev.path, &ns, &mut known_entities, &mut bindings)?;
                    let sid = StableId::derive(&["file", &ev.path]);
                    observe(store, &sid, "present", "false", commit.at_ms)?;
                    deleted.insert(ev.path.clone());
                    report.deletions += 1;
                }
                'R' => {
                    // A rename is a deletion plus an appearance, joined by
                    // a renamed_to edge so the identity trail survives.
                    if ingestible(&ev.path) {
                        ensure_entity(
                            store, prefix, &ev.path, &ns, &mut known_entities, &mut bindings,
                        )?;
                        let old = StableId::derive(&["file", &ev.path]);
                        observe(store, &old, "present", "false", commit.at_ms)?;
                        deleted.insert(ev.path.clone());
                        report.deletions += 1;
                    }
                    if let Some(new_path) = &ev.renamed_to {
                        if ingestible(&ev.path) && ingestible(new_path) {
                            store.put(&Object::Relation {
                                from: StableId::derive(&["file", &ev.path]),
                                predicate: "renamed_to".to_string(),
                                to: StableId::derive(&["file", new_path]),
                                source: "backfill".to_string(),
                                observed_at_ms: commit.at_ms,
                            })?;
                        }
                        record_version(
                            store, root, prefix, &commit.hash, new_path, commit.at_ms, &ns,
                            &mut known_entities, &mut bindings, &mut deleted, &mut report,
                        )?;
                    }
                }
                _ => {
                    record_version(
                        store, root, prefix, &commit.hash, &ev.path, commit.at_ms, &ns,
                        &mut known_entities, &mut bindings, &mut deleted, &mut report,
                    )?;
                }
            }
        }
    }

    if !bindings.is_empty() {
        store.bind_many(bindings)?;
    }
    report.objects_written = store.count_objects()?.saturating_sub(before);
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn record_version(
    store: &Store,
    root: &Path,
    prefix: &str,
    commit: &str,
    path: &str,
    at_ms: u64,
    ns: &BTreeMap<String, NodeId>,
    known: &mut BTreeSet<String>,
    bindings: &mut Vec<(String, NodeId)>,
    deleted: &mut BTreeSet<String>,
    report: &mut BackfillReport,
) -> Result<(), StoreError> {
    if !ingestible(path) {
        return Ok(());
    }
    let out = Command::new("git")
        .args(["-C"])
        .arg(root)
        .args(["-c", "core.quotepath=false", "show", &format!("{commit}:{path}")])
        .output()?;
    if !out.status.success() {
        return Ok(()); // vanished from git's view (submodule, mode change)
    }
    if out.stdout.len() > MAX_BLOB {
        report.skipped_blobs += 1;
        return Ok(());
    }
    ensure_entity(store, prefix, path, ns, known, bindings)?;
    let sid = StableId::derive(&["file", path]);
    let hash = blake3::hash(&out.stdout).to_hex().to_string();
    observe(store, &sid, "content_b3", &hash, at_ms)?;
    report.file_versions += 1;
    // A file reappearing after a historical deletion is present again.
    if deleted.remove(path) {
        observe(store, &sid, "present", "true", at_ms)?;
    }
    Ok(())
}

fn ensure_entity(
    store: &Store,
    prefix: &str,
    path: &str,
    ns: &BTreeMap<String, NodeId>,
    known: &mut BTreeSet<String>,
    bindings: &mut Vec<(String, NodeId)>,
) -> Result<(), StoreError> {
    if !known.insert(path.to_string()) {
        return Ok(());
    }
    let mut labels = BTreeMap::new();
    labels.insert("path".to_string(), path.to_string());
    let node = store.put(&Object::Entity {
        id: StableId::derive(&["file", path]),
        entity_kind: "source_file".to_string(),
        labels,
    })?;
    let name = format!("{prefix}/{path}");
    if !ns.contains_key(&name) && !bindings.iter().any(|(n, _)| n == &name) {
        bindings.push((name, node));
    }
    Ok(())
}

fn observe(
    store: &Store,
    subject: &StableId,
    property: &str,
    value: &str,
    at_ms: u64,
) -> Result<(), StoreError> {
    // No latest() guard needed: identical facts are content-addressed
    // no-ops, and distinct historical values are the timeline itself.
    store.put(&Object::Observation {
        subject: subject.clone(),
        property: property.to_string(),
        value: value.to_string(),
        source: "backfill".to_string(),
        observed_at_ms: at_ms,
    })?;
    Ok(())
}

fn ingestible(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    if INGEST_FILENAMES.contains(&name) {
        return true;
    }
    path.rsplit('.')
        .next()
        .is_some_and(|ext| INGEST_EXTENSIONS.contains(&ext))
}

/// One `git log` call: every commit oldest-first with its changed files.
fn read_history(root: &Path) -> Result<Vec<CommitEntry>, StoreError> {
    let out = Command::new("git")
        .args(["-C"])
        .arg(root)
        .args([
            "-c",
            "core.quotepath=false",
            "log",
            "--reverse",
            "--date-order",
            "--format=@@%H|%ct",
            "--name-status",
        ])
        .output()?;
    if !out.status.success() {
        return Err(StoreError::Io(std::io::Error::other(format!(
            "git log failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ))));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut commits: Vec<CommitEntry> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("@@") {
            let (hash, epoch) = rest.split_once('|').unwrap_or((rest, "0"));
            commits.push(CommitEntry {
                hash: hash.to_string(),
                at_ms: epoch.trim().parse::<u64>().unwrap_or(0) * 1000,
                events: Vec::new(),
            });
            continue;
        }
        let mut parts = line.split('\t');
        let (Some(status), Some(path)) = (parts.next(), parts.next()) else { continue };
        let Some(current) = commits.last_mut() else { continue };
        let s = status.chars().next().unwrap_or(' ');
        if !matches!(s, 'A' | 'M' | 'D' | 'R' | 'C' | 'T') {
            continue;
        }
        current.events.push(FileEvent {
            status: if s == 'C' { 'A' } else { s },
            path: path.to_string(),
            renamed_to: parts.next().map(str::to_string),
        });
    }
    Ok(commits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::twin::{files_at, latest, refresh};
    use brain_index::{replay, Index as _, MemIndex};
    use std::fs;

    fn git(dir: &Path, epoch: u64, args: &[&str]) {
        let date = format!("@{epoch} +0000");
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .env("GIT_AUTHOR_DATE", &date)
                .env("GIT_COMMITTER_DATE", &date)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap()
                .status
                .success(),
            "git {args:?}"
        );
    }

    #[test]
    fn history_arrives_with_churn_timelines_and_resurrections() {
        let repo = tempfile::tempdir().unwrap();
        let d = repo.path();
        git(d, 1_000_000, &["init", "-q"]);
        fs::write(d.join("a.rs"), "v1\n").unwrap();
        fs::write(d.join("b.rs"), "b\n").unwrap();
        git(d, 1_000_000, &["add", "."]);
        git(d, 1_000_000, &["commit", "-qm", "c1"]);
        fs::write(d.join("a.rs"), "v2\n").unwrap();
        git(d, 2_000_000, &["commit", "-qam", "c2"]);
        fs::remove_file(d.join("b.rs")).unwrap();
        git(d, 3_000_000, &["add", "-A"]);
        git(d, 3_000_000, &["commit", "-qm", "c3 delete b"]);
        fs::write(d.join("b.rs"), "b returns\n").unwrap();
        git(d, 4_000_000, &["add", "."]);
        git(d, 4_000_000, &["commit", "-qm", "c4 resurrect b"]);

        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        let report = backfill(&store, d, "twin/old", 0).unwrap();
        assert_eq!(report.commits, 4);
        assert_eq!(report.file_versions, 4, "a.rs x2 + b.rs x2");
        assert_eq!(report.deletions, 1);

        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        let a = StableId::derive(&["file", "a.rs"]);
        let b = StableId::derive(&["file", "b.rs"]);

        // Timelines carry commit-time stamps: as-of works across history.
        let at_c1 = files_at(&store, &index, "twin/old", 1_000_000_000).unwrap();
        assert_eq!(at_c1.len(), 2, "{at_c1:?}");
        let at_c3 = files_at(&store, &index, "twin/old", 3_000_000_000).unwrap();
        assert!(!at_c3.iter().any(|(r, _)| r == "b.rs"), "b deleted at c3");
        let at_c4 = files_at(&store, &index, "twin/old", 4_000_000_000).unwrap();
        assert!(at_c4.iter().any(|(r, _)| r == "b.rs"), "b resurrected at c4");

        // Churn counts real history.
        let a_versions = index
            .observations_of(&a)
            .iter()
            .filter_map(|id| store.get(id).ok())
            .filter(|o| matches!(o, Object::Observation { property, .. } if property == "content_b3"))
            .count();
        assert_eq!(a_versions, 2);
        assert_eq!(latest(&index, &store, &b, "present").unwrap().as_deref(), Some("true"));

        // Idempotent: a second backfill writes nothing at all.
        let before = store.count_objects().unwrap();
        let again = backfill(&store, d, "twin/old", 0).unwrap();
        assert_eq!(again.objects_written, 0, "{again:?}");
        assert_eq!(store.count_objects().unwrap(), before);

        // A live refresh composes on top: current state wins timelines.
        refresh(&store, d, "twin/old").unwrap();
        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        let now_files = files_at(&store, &index, "twin/old", u64::MAX).unwrap();
        assert_eq!(now_files.len(), 2);

        // Repo-level: every commit is a git_commit observation, so
        // `twin at <hash>` can resolve any point in history.
        let repo_sid = StableId::derive(&["repo", "twin/old"]);
        let commits_observed = index
            .observations_of(&repo_sid)
            .iter()
            .filter_map(|id| store.get(id).ok())
            .filter(|o| matches!(o, Object::Observation { property, .. } if property == "git_commit"))
            .count();
        assert!(commits_observed >= 4);
    }
}
