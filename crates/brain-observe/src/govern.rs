//! Governed mode: the motor system — brain mediates changes to the
//! external software it twins, through the intent/receipt boundary.
//!
//! Lifecycle: **propose** (pure graph write — nothing touches disk) →
//! **apply** (durable Intent BEFORE the write, the write, then Receipt) →
//! **verify** (run the repo's test command, link the protocol) →
//! optionally **revert** (another governed write, back to the recorded
//! before-state). Every mutation leaves reason, before/after hashes,
//! intent, receipt, and verification in the graph.
//!
//! No ambient authority: apply and revert refuse without the `fs`
//! capability, exactly as runtime effects refuse without theirs. A crash
//! between intent and receipt leaves the change *indeterminate* — marked
//! by reconcile, never blindly retried.

use crate::twin::{latest, observe_src, relate};
use brain_core::ids::{NodeId, StableId};
use brain_core::object::Object;
use brain_index::{replay, Index, MemIndex};
use brain_store::intents::IntentState;
use brain_store::{now_ms, Store, StoreError};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

pub fn change_sid(prefix: &str, slug: &str) -> StableId {
    StableId::derive(&["change", prefix, slug])
}

#[derive(Debug)]
pub struct Proposal {
    pub slug: String,
    pub sid: StableId,
    pub before_b3: Option<String>,
    pub after_b3: String,
    /// False when this exact proposal already existed (idempotent).
    pub wrote: bool,
}

/// Propose a governed change: record what would be written and why.
/// Pure graph write; the working tree is untouched.
pub fn propose(
    store: &Store,
    root: &Path,
    prefix: &str,
    rel_path: &str,
    new_content: &str,
    reason: &str,
) -> Result<Proposal, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    let now = now_ms();
    let after_b3 = blake3::hash(new_content.as_bytes()).to_hex().to_string();
    let stem = rel_path.rsplit('/').next().unwrap_or(rel_path);
    let slug = format!("{}-{}", stem.to_lowercase(), &after_b3[..8]);
    let sid = change_sid(prefix, &slug);
    let existed = latest(&index, store, &sid, "status")?.is_some();

    let before = fs::read_to_string(root.join(rel_path)).ok();
    let before_b3 = before.as_ref().map(|c| blake3::hash(c.as_bytes()).to_hex().to_string());

    let mut labels = BTreeMap::new();
    labels.insert("prefix".to_string(), prefix.to_string());
    labels.insert("slug".to_string(), slug.clone());
    labels.insert("target".to_string(), rel_path.to_string());
    labels.insert("title".to_string(), reason.to_string());
    store.put(&Object::Entity { id: sid.clone(), entity_kind: "change".to_string(), labels })?;

    let mut props: Vec<(&str, &str)> = vec![
        ("target", rel_path),
        ("reason", reason),
        ("content", new_content),
        ("after_b3", &after_b3),
    ];
    if let Some(b) = &before {
        props.push(("before_content", b));
    }
    let before_hash = before_b3.clone().unwrap_or_else(|| "absent".to_string());
    props.push(("before_b3", &before_hash));
    for (prop, value) in props {
        if latest(&index, store, &sid, prop)?.as_deref() != Some(value) {
            observe_src(store, &sid, prop, value, "govern", now)?;
        }
    }
    if !existed {
        observe_src(store, &sid, "status", "proposed", "govern", now)?;
    }
    let mut written = BTreeSet::new();
    let file_sid = StableId::derive(&["file", rel_path]);
    relate(store, &index, &mut written, &sid, "changes", &file_sid, now)?;
    let repo_sid = StableId::derive(&["repo", prefix]);
    relate(store, &index, &mut written, &sid, "concerns", &repo_sid, now)?;

    Ok(Proposal { slug, sid, before_b3, after_b3, wrote: !existed })
}

/// Propose a governed move (tidy's archive path): rename `from_rel` to
/// `to_rel`. Works for files and whole directories (prototypes). The
/// before-hash is recorded for files so the trail stays verifiable;
/// reverting renames back.
pub fn propose_move(
    store: &Store,
    root: &Path,
    prefix: &str,
    from_rel: &str,
    to_rel: &str,
    reason: &str,
) -> Result<Proposal, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    let now = now_ms();
    let key = blake3::hash(format!("{from_rel}->{to_rel}").as_bytes()).to_hex().to_string();
    let stem = from_rel.rsplit('/').next().unwrap_or(from_rel);
    let slug = format!("{}-mv-{}", stem.to_lowercase(), &key[..8]);
    let sid = change_sid(prefix, &slug);
    let existed = latest(&index, store, &sid, "status")?.is_some();

    let before_b3 = fs::read(root.join(from_rel))
        .ok()
        .map(|b| blake3::hash(&b).to_hex().to_string());

    let mut labels = BTreeMap::new();
    labels.insert("prefix".to_string(), prefix.to_string());
    labels.insert("slug".to_string(), slug.clone());
    labels.insert("target".to_string(), from_rel.to_string());
    labels.insert("title".to_string(), reason.to_string());
    store.put(&Object::Entity { id: sid.clone(), entity_kind: "change".to_string(), labels })?;

    let mut props: Vec<(&str, &str)> =
        vec![("target", from_rel), ("move_to", to_rel), ("reason", reason)];
    let before_hash = before_b3.clone().unwrap_or_else(|| "dir".to_string());
    props.push(("before_b3", &before_hash));
    for (prop, value) in props {
        if latest(&index, store, &sid, prop)?.as_deref() != Some(value) {
            observe_src(store, &sid, prop, value, "govern", now)?;
        }
    }
    if !existed {
        observe_src(store, &sid, "status", "proposed", "govern", now)?;
    }
    let mut written = BTreeSet::new();
    let file_sid = StableId::derive(&["file", from_rel]);
    relate(store, &index, &mut written, &sid, "changes", &file_sid, now)?;
    let repo_sid = StableId::derive(&["repo", prefix]);
    relate(store, &index, &mut written, &sid, "concerns", &repo_sid, now)?;

    Ok(Proposal {
        slug,
        sid,
        before_b3,
        after_b3: "moved".to_string(),
        wrote: !existed,
    })
}

#[derive(Debug)]
pub struct Applied {
    pub intent: NodeId,
    pub receipt: NodeId,
    pub ok: bool,
}

/// Apply a proposed change through the effect boundary. Refuses without
/// the `fs` capability — no ambient authority, same as runtime effects.
pub fn apply(
    store: &Store,
    root: &Path,
    prefix: &str,
    slug: &str,
    caps: &[String],
) -> Result<Applied, StoreError> {
    perform(store, root, prefix, slug, caps, false)
}

/// Revert an applied change: a governed write of the recorded before-state
/// (or a governed removal when the change created the file).
pub fn revert(
    store: &Store,
    root: &Path,
    prefix: &str,
    slug: &str,
    caps: &[String],
) -> Result<Applied, StoreError> {
    perform(store, root, prefix, slug, caps, true)
}

fn perform(
    store: &Store,
    root: &Path,
    prefix: &str,
    slug: &str,
    caps: &[String],
    reverting: bool,
) -> Result<Applied, StoreError> {
    if !caps.iter().any(|c| c == "fs") {
        return Err(StoreError::Io(std::io::Error::other(
            "refused: governed writes require --cap fs (no ambient authority)",
        )));
    }
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    let now = now_ms();
    let sid = change_sid(prefix, slug);
    let status = latest(&index, store, &sid, "status")?
        .ok_or_else(|| StoreError::Io(std::io::Error::other(format!("no change '{slug}'"))))?;
    let expected = if reverting { "applied" } else { "proposed" };
    if status != expected && !(reverting && status == "verified") {
        return Err(StoreError::Io(std::io::Error::other(format!(
            "change '{slug}' is '{status}', expected '{expected}'"
        ))));
    }
    let target = latest(&index, store, &sid, "target")?
        .ok_or_else(|| StoreError::Io(std::io::Error::other("change has no target")))?;
    let move_to = latest(&index, store, &sid, "move_to")?;
    let (action, payload) = if let Some(dest) = &move_to {
        // A governed move: apply renames target -> dest, revert renames back.
        let route = if reverting {
            format!("{dest}\u{0}{target}")
        } else {
            format!("{target}\u{0}{dest}")
        };
        ("fs/rename", Some(route))
    } else if reverting {
        match latest(&index, store, &sid, "before_content")? {
            Some(before) => ("fs/write", Some(before)),
            None => ("fs/remove", None), // the change created the file
        }
    } else {
        let content = latest(&index, store, &sid, "content")?
            .ok_or_else(|| StoreError::Io(std::io::Error::other("change has no content")))?;
        ("fs/write", Some(content))
    };

    // 1. Intent, durably logged BEFORE the effect.
    let arg_hash = brain_core::canonical::hash_bytes(
        payload.as_deref().unwrap_or(&target).as_bytes(),
    );
    let intent = store.put(&Object::Intent {
        action: action.to_string(),
        arg_hash,
        capability: Some("fs".to_string()),
        at_ms: now,
    })?;
    store.intents().begin(intent)?;
    observe_src(store, &sid, "intent", &intent.to_string(), "govern", now)?;

    // 2. The effect.
    let path = root.join(&target);
    let result: Result<(), std::io::Error> = (|| {
        if action == "fs/rename" {
            let route = payload.as_deref().unwrap_or_default();
            let (from, to) = route
                .split_once('\u{0}')
                .ok_or_else(|| std::io::Error::other("malformed move route"))?;
            let to_path = root.join(to);
            if let Some(parent) = to_path.parent() {
                fs::create_dir_all(parent)?;
            }
            return fs::rename(root.join(from), &to_path);
        }
        match &payload {
            Some(content) => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let tmp = path.with_extension("brain-tmp");
                fs::write(&tmp, content)?;
                fs::rename(&tmp, &path)
            }
            None => fs::remove_file(&path),
        }
    })();

    // 3. Receipt, whatever happened.
    let ok = result.is_ok();
    let detail = match &result {
        Ok(()) => format!("{action} {target}"),
        Err(e) => format!("{action} {target} failed: {e}"),
    };
    let receipt = store.put(&Object::Receipt { intent, ok, detail, at_ms: now_ms() })?;
    if ok {
        store.intents().confirm(intent, receipt)?;
    } else {
        store.intents().fail(intent, receipt)?;
    }
    let new_status = match (ok, reverting) {
        (true, false) => "applied",
        (true, true) => "reverted",
        (false, _) => "failed",
    };
    observe_src(store, &sid, "status", new_status, "govern", now_ms())?;
    Ok(Applied { intent, receipt, ok })
}

#[derive(Debug)]
pub struct Verification {
    pub passed: bool,
    pub total: usize,
    pub failed: usize,
}

/// Verify an applied change: run the repo's graph-configured test command,
/// import the protocol, and link it to the change.
pub fn verify(
    store: &Store,
    root: &Path,
    prefix: &str,
    slug: &str,
) -> Result<Verification, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    let now = now_ms();
    let sid = change_sid(prefix, slug);
    let repo_sid = StableId::derive(&["repo", prefix]);
    let cmd = latest(&index, store, &repo_sid, "test_command")?.ok_or_else(|| {
        StoreError::Io(std::io::Error::other(
            "no test command stored — run `brain hook install --tests` first",
        ))
    })?;
    let out = std::process::Command::new("sh")
        .args(["-c", &cmd])
        .current_dir(root)
        .output()?;
    let mut raw = String::from_utf8_lossy(&out.stdout).into_owned();
    raw.push_str(&String::from_utf8_lossy(&out.stderr));
    let report = crate::testing::parse_report(&raw);
    let outcome = crate::testing::record_run(store, prefix, &report, &raw)?;
    let mut written = BTreeSet::new();
    relate(store, &index, &mut written, &sid, "verified_by", &outcome.run_sid, now)?;
    let passed = outcome.failed == 0 && outcome.total > 0;
    let status = if passed { "verified" } else { "broken" };
    if latest(&index, store, &sid, "status")?.as_deref() != Some(status) {
        observe_src(store, &sid, "status", status, "govern", now)?;
    }
    Ok(Verification { passed, total: outcome.total, failed: outcome.failed })
}

/// Reconciliation after recovery: changes whose intent the log marked
/// indeterminate get an indeterminate status observation. Marks only —
/// never re-executes; deciding what really happened is the caller's
/// deliberate act.
pub fn reconcile(store: &Store, prefix: &str) -> Result<Vec<String>, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    let states = store.intents().states()?;
    let now = now_ms();
    let mut marked = Vec::new();
    let mut seen = BTreeSet::new();
    for node in index.entities_by_kind("change") {
        let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else { continue };
        if labels.get("prefix").map(String::as_str) != Some(prefix) || !seen.insert(id.clone()) {
            continue;
        }
        let Some(intent) = latest(&index, store, &id, "intent")? else { continue };
        if states.get(&intent) == Some(&IntentState::Indeterminate)
            && latest(&index, store, &id, "status")?.as_deref() != Some("indeterminate")
        {
            observe_src(store, &id, "status", "indeterminate", "govern", now)?;
            marked.push(labels.get("slug").cloned().unwrap_or_default());
        }
    }
    Ok(marked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::twin::refresh;

    fn setup() -> (tempfile::TempDir, tempfile::TempDir, Store) {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::write(src.path().join("src/main.rs"), "pub fn main() {}\n").unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        (src, store_dir, store)
    }

    #[test]
    fn propose_is_pure_and_apply_requires_capability() {
        let (src, _sd, store) = setup();
        let p = propose(
            &store,
            src.path(),
            "twin/app",
            "src/main.rs",
            "pub fn main() { improved() }\n",
            "improve main",
        )
        .unwrap();
        assert!(p.wrote);
        assert!(p.before_b3.is_some());
        // Disk untouched by a proposal.
        assert_eq!(
            fs::read_to_string(src.path().join("src/main.rs")).unwrap(),
            "pub fn main() {}\n"
        );
        // No ambient authority: apply without the capability is refused
        // and the working tree stays untouched.
        let err = apply(&store, src.path(), "twin/app", &p.slug, &[]).unwrap_err();
        assert!(err.to_string().contains("no ambient authority"), "{err}");
        assert_eq!(
            fs::read_to_string(src.path().join("src/main.rs")).unwrap(),
            "pub fn main() {}\n"
        );
    }

    #[test]
    fn apply_leaves_intent_receipt_trail_and_revert_restores() {
        let (src, _sd, store) = setup();
        let p = propose(
            &store,
            src.path(),
            "twin/app",
            "src/main.rs",
            "pub fn main() { improved() }\n",
            "improve main",
        )
        .unwrap();
        let caps = vec!["fs".to_string()];
        let a = apply(&store, src.path(), "twin/app", &p.slug, &caps).unwrap();
        assert!(a.ok);
        assert_eq!(
            fs::read_to_string(src.path().join("src/main.rs")).unwrap(),
            "pub fn main() { improved() }\n"
        );
        // The trail: intent confirmed in the durable log, receipt stored,
        // status applied.
        let states = store.intents().states().unwrap();
        assert_eq!(states.get(&a.intent.to_string()), Some(&IntentState::Confirmed));
        match store.get(&a.receipt).unwrap() {
            Object::Receipt { intent, ok, .. } => {
                assert_eq!(intent, a.intent);
                assert!(ok);
            }
            other => panic!("expected receipt, got {other:?}"),
        }
        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        let sid = change_sid("twin/app", &p.slug);
        assert_eq!(latest(&index, &store, &sid, "status").unwrap().as_deref(), Some("applied"));
        // The twin sees the governed change as ordinary drift on refresh.
        let r = refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(r.changed, vec!["src/main.rs".to_string()]);

        // Revert: a second governed write, back to the recorded before.
        let rv = revert(&store, src.path(), "twin/app", &p.slug, &caps).unwrap();
        assert!(rv.ok);
        assert_eq!(
            fs::read_to_string(src.path().join("src/main.rs")).unwrap(),
            "pub fn main() {}\n"
        );
        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        assert_eq!(latest(&index, &store, &sid, "status").unwrap().as_deref(), Some("reverted"));
    }

    #[test]
    fn verify_links_a_protocol_and_grades_the_change() {
        let (src, _sd, store) = setup();
        // Store a test command on the repo entity (what --tests installs).
        let repo = StableId::derive(&["repo", "twin/app"]);
        observe_src(
            &store,
            &repo,
            "test_command",
            "printf 'test a::t ... ok\\n'",
            "hook",
            now_ms(),
        )
        .unwrap();
        let p = propose(&store, src.path(), "twin/app", "src/main.rs", "pub fn main() { v2() }\n", "v2")
            .unwrap();
        apply(&store, src.path(), "twin/app", &p.slug, &["fs".to_string()]).unwrap();
        let v = verify(&store, src.path(), "twin/app", &p.slug).unwrap();
        assert!(v.passed);
        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        let sid = change_sid("twin/app", &p.slug);
        assert_eq!(latest(&index, &store, &sid, "status").unwrap().as_deref(), Some("verified"));
        assert_eq!(index.relations_from(&sid, "verified_by").len(), 1);
    }

    #[test]
    fn crash_between_intent_and_receipt_reconciles_to_indeterminate() {
        let (src, _sd, store) = setup();
        let p = propose(&store, src.path(), "twin/app", "src/main.rs", "pub fn main() { x() }\n", "x")
            .unwrap();
        // Simulate the crash window: intent begun and recorded on the
        // change, no receipt ever written.
        let now = now_ms();
        let intent = store
            .put(&Object::Intent {
                action: "fs/write".to_string(),
                arg_hash: brain_core::canonical::hash_bytes(b"x"),
                capability: Some("fs".to_string()),
                at_ms: now,
            })
            .unwrap();
        store.intents().begin(intent).unwrap();
        let sid = change_sid("twin/app", &p.slug);
        observe_src(&store, &sid, "intent", &intent.to_string(), "govern", now).unwrap();

        // Recovery marks the intent; reconcile marks the change. Nothing
        // is retried, the working tree is untouched.
        store.intents().recover().unwrap();
        let marked = reconcile(&store, "twin/app").unwrap();
        assert_eq!(marked, vec![p.slug.clone()]);
        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        assert_eq!(
            latest(&index, &store, &sid, "status").unwrap().as_deref(),
            Some("indeterminate")
        );
        assert_eq!(
            fs::read_to_string(src.path().join("src/main.rs")).unwrap(),
            "pub fn main() {}\n"
        );
        // Reconcile is idempotent.
        assert!(reconcile(&store, "twin/app").unwrap().is_empty());
    }
}
