//! The twin engine: continuous, drift-aware reflection of external software.
//!
//! `refresh` is the sense organ: it walks a source tree, compares reality
//! against the twin's latest claims, and records only what changed — new and
//! changed files get fresh observations, structure (symbols via
//! [`crate::symbols`]) and relations; vanished files get `present=false`;
//! unchanged files write nothing, so repeated refreshes do not grow the
//! graph. `status` runs the identical comparison read-only.
//!
//! Nothing is ever overwritten: the twin's history of a file is its
//! observation timeline, and "deleted" is itself just an observation.

use crate::collect_files;
use crate::symbols;
use brain_core::ids::{NodeId, StableId};
use brain_core::object::Object;
use brain_index::{replay, Index, MemIndex};
use brain_store::{now_ms, Store, StoreError};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Default, PartialEq)]
pub struct TwinReport {
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub deleted: Vec<String>,
    pub unchanged: usize,
    /// Symbol entities written this run (refresh only).
    pub symbols: usize,
    /// Relations written this run (refresh only).
    pub relations: usize,
}

/// Refresh the twin under `prefix` from the tree at `root`, writing only
/// what drifted. Idempotent: an immediately repeated refresh writes nothing.
pub fn refresh(store: &Store, root: &Path, prefix: &str) -> Result<TwinReport, StoreError> {
    run(store, root, prefix, true)
}

/// The same comparison as [`refresh`], read-only: what *would* be recorded?
pub fn status(store: &Store, root: &Path, prefix: &str) -> Result<TwinReport, StoreError> {
    run(store, root, prefix, false)
}

fn run(store: &Store, root: &Path, prefix: &str, write: bool) -> Result<TwinReport, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    let now = now_ms();
    let mut report = TwinReport::default();

    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort();
    let file_set: BTreeSet<String> = files.iter().cloned().collect();

    let ns = store.namespace()?;
    let known: BTreeSet<String> = ns
        .keys()
        .filter_map(|n| n.strip_prefix(&format!("{prefix}/")))
        .map(str::to_string)
        .collect();

    let mut bindings: Vec<(String, NodeId)> = Vec::new();
    let mut written_relations: BTreeSet<(StableId, String, StableId)> = BTreeSet::new();

    for rel in &files {
        let content = fs::read(root.join(rel))?;
        let hash = blake3::hash(&content).to_hex().to_string();
        let sid = StableId::derive(&["file", rel]);
        let prior = latest(&index, store, &sid, "content_b3")?;

        let changed = match &prior {
            None => {
                report.added.push(rel.clone());
                true
            }
            Some(v) if *v != hash => {
                report.changed.push(rel.clone());
                true
            }
            _ => {
                report.unchanged += 1;
                false
            }
        };

        if !write {
            continue;
        }

        // A file that was marked deleted and is back gets its presence restored
        // even when its content is unchanged.
        if latest(&index, store, &sid, "present")?.as_deref() == Some("false") {
            observe(store, &sid, "present", "true", now)?;
        }

        let text = String::from_utf8_lossy(&content);
        let structure = symbols::analyze(rel, &text);
        // Backfill: an unchanged file twinned before structure extraction
        // existed (no `language` observation yet) still gets its structure.
        let structure_missing = !structure.language.is_empty()
            && latest(&index, store, &sid, "language")?.is_none();
        if !changed && !structure_missing {
            continue;
        }

        if changed {
            let mut labels = BTreeMap::new();
            labels.insert("path".to_string(), rel.clone());
            let entity_node = store.put(&Object::Entity {
                id: sid.clone(),
                entity_kind: "source_file".to_string(),
                labels,
            })?;
            if prior.is_none() {
                bindings.push((format!("{prefix}/{rel}"), entity_node));
            }
            observe(store, &sid, "content_b3", &hash, now)?;
        }

        if !structure.language.is_empty()
            && latest(&index, store, &sid, "language")?.as_deref() != Some(structure.language)
        {
            observe(store, &sid, "language", structure.language, now)?;
        }

        for sym in &structure.symbols {
            let sym_sid = StableId::derive(&["symbol", rel, sym.kind, &sym.name]);
            let mut labels = BTreeMap::new();
            labels.insert("file".to_string(), rel.clone());
            labels.insert("kind".to_string(), sym.kind.to_string());
            labels.insert("name".to_string(), sym.name.clone());
            store.put(&Object::Entity {
                id: sym_sid.clone(),
                entity_kind: "symbol".to_string(),
                labels,
            })?;
            report.symbols += 1;
            if latest(&index, store, &sym_sid, "line")?.as_deref()
                != Some(sym.line.to_string().as_str())
            {
                observe(store, &sym_sid, "line", &sym.line.to_string(), now)?;
            }
            if relate(store, &index, &mut written_relations, &sid, "contains", &sym_sid, now)? {
                report.relations += 1;
            }
        }

        for import in &structure.imports {
            let target = match resolve_import(rel, import, &file_set) {
                Some(target_rel) => StableId::derive(&["file", &target_rel]),
                None => {
                    let module_sid = StableId::derive(&["module", import]);
                    let mut labels = BTreeMap::new();
                    labels.insert("name".to_string(), import.clone());
                    store.put(&Object::Entity {
                        id: module_sid.clone(),
                        entity_kind: "module".to_string(),
                        labels,
                    })?;
                    module_sid
                }
            };
            if relate(store, &index, &mut written_relations, &sid, "imports", &target, now)? {
                report.relations += 1;
            }
        }
    }

    // Files the twin still claims are present but which are gone from disk.
    for rel in known.iter() {
        if file_set.contains(rel) {
            continue;
        }
        let sid = StableId::derive(&["file", rel]);
        if latest(&index, store, &sid, "present")?.as_deref() == Some("false") {
            continue; // already recorded; no drift
        }
        report.deleted.push(rel.clone());
        if write {
            observe(store, &sid, "present", "false", now)?;
        }
    }

    if write {
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
        for (prop, value) in git_info(root) {
            if latest(&index, store, &repo_sid, &prop)?.as_deref() != Some(value.as_str()) {
                observe(store, &repo_sid, &prop, &value, now)?;
            }
        }
        if !bindings.is_empty() {
            store.bind_many(bindings)?;
        }

        // Record the totals series on the repo entity: files/symbols/relations
        // over time, guarded so an unchanged codebase writes nothing. This is
        // what makes insights *continuous* — trends persist in the graph and
        // travel with replication.
        let mut post = MemIndex::new();
        replay(store, &mut post)?;
        let ins = insights_with(store, &post, prefix)?;
        let totals = [
            ("files_present", ins.files),
            ("symbols_total", ins.symbols),
            ("relations_total", ins.relations),
        ];
        let mut any_changed = false;
        for (prop, value) in totals {
            if latest(&post, store, &repo_sid, prop)?.as_deref() != Some(value.to_string().as_str())
            {
                any_changed = true;
            }
        }
        // Write all three together so every series point is complete.
        if any_changed {
            for (prop, value) in totals {
                observe(store, &repo_sid, prop, &value.to_string(), now)?;
            }
        }
    }

    Ok(report)
}

// ---------------------------------------------------------------------------
// Insights: a synthesized picture of the software under a twin prefix
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct Insights {
    pub files: usize,
    pub deleted_files: usize,
    pub symbols: usize,
    pub relations: usize,
    /// External dependencies (unresolved imports): (module, importer count).
    pub external_modules: Vec<(String, usize)>,
    /// Most-edited files since twinning: (path, observed versions > 1).
    pub churn: Vec<(String, usize)>,
    /// Most-imported files: (path, importer count).
    pub hubs: Vec<(String, usize)>,
    /// Largest files by declared symbols: (path, symbol count).
    pub largest: Vec<(String, usize)>,
    /// Most recent agent notes: (at_ms, entity path, text), newest first.
    pub notes: Vec<(u64, String, String)>,
    pub git_commit: Option<String>,
    pub git_branch: Option<String>,
    /// Growth series from the repo entity, oldest first: (at_ms, files,
    /// symbols, relations) — one point per refresh that changed the totals.
    pub series: Vec<(u64, usize, usize, usize)>,
}

const TOP: usize = 5;

pub fn insights(store: &Store, prefix: &str) -> Result<Insights, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    insights_with(store, &index, prefix)
}

pub fn insights_with(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
) -> Result<Insights, StoreError> {
    let mut ins = Insights::default();
    let ns = store.namespace()?;
    let mut file_sids: Vec<(String, StableId)> = Vec::new();

    for (name, node) in &ns {
        let Some(rel) = name.strip_prefix(&format!("{prefix}/")) else { continue };
        if let Ok(Object::Entity { id, entity_kind, .. }) = store.get(node) {
            if entity_kind == "source_file" {
                file_sids.push((rel.to_string(), id));
            }
        }
    }

    let mut churn: Vec<(String, usize)> = Vec::new();
    let mut hubs: Vec<(String, usize)> = Vec::new();
    let mut largest: Vec<(String, usize)> = Vec::new();
    let mut modules: BTreeMap<String, usize> = BTreeMap::new();

    for (rel, sid) in &file_sids {
        if latest(index, store, sid, "present")?.as_deref() == Some("false") {
            ins.deleted_files += 1;
            continue;
        }
        ins.files += 1;

        let versions = index
            .observations_of(sid)
            .iter()
            .filter_map(|id| store.get(id).ok())
            .filter(|o| matches!(o, Object::Observation { property, .. } if property == "content_b3"))
            .count();
        if versions > 1 {
            churn.push((rel.clone(), versions));
        }

        let contains = index.relations_from(sid, "contains").len();
        ins.relations += contains;
        if contains > 0 {
            largest.push((rel.clone(), contains));
        }
        ins.symbols += contains;

        let importers = index.relations_to(sid, "imports").len();
        if importers > 0 {
            hubs.push((rel.clone(), importers));
        }

        for id in index.relations_from(sid, "imports") {
            ins.relations += 1;
            if let Object::Relation { to, .. } = store.get(&id)? {
                for node in index.entity_nodes(&to) {
                    if let Ok(Object::Entity { entity_kind, labels, .. }) = store.get(&node) {
                        if entity_kind == "module" {
                            let name = labels.get("name").cloned().unwrap_or_default();
                            *modules.entry(name).or_default() += 1;
                        }
                        break;
                    }
                }
            }
        }
    }

    let top = |mut v: Vec<(String, usize)>| {
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v.truncate(TOP);
        v
    };
    ins.churn = top(churn);
    ins.hubs = top(hubs);
    ins.largest = top(largest);
    ins.external_modules = top(modules.into_iter().collect());

    // Notes across all twinned files plus the repo entity, newest first.
    let repo_sid = StableId::derive(&["repo", prefix]);
    let mut note_subjects = file_sids.clone();
    note_subjects.push((prefix.to_string(), repo_sid.clone()));
    let mut all_notes: Vec<(u64, String, String)> = Vec::new();
    for (rel, sid) in &note_subjects {
        for (at, text) in notes(index, store, sid)? {
            all_notes.push((at, rel.clone(), text));
        }
    }
    all_notes.sort_by(|a, b| b.0.cmp(&a.0));
    all_notes.truncate(TOP);
    ins.notes = all_notes;

    ins.git_commit = latest(index, store, &repo_sid, "git_commit")?;
    ins.git_branch = latest(index, store, &repo_sid, "git_branch")?;

    // Growth series: pair up the repo entity's totals observations by time.
    let mut points: BTreeMap<u64, (usize, usize, usize)> = BTreeMap::new();
    for id in index.observations_of(&repo_sid) {
        if let Object::Observation { property, value, observed_at_ms, .. } = store.get(&id)? {
            let Ok(n) = value.parse::<usize>() else { continue };
            let point = points.entry(observed_at_ms).or_insert((0, 0, 0));
            match property.as_str() {
                "files_present" => point.0 = n,
                "symbols_total" => point.1 = n,
                "relations_total" => point.2 = n,
                _ => {}
            }
        }
    }
    ins.series = points
        .into_iter()
        .filter(|(_, (f, s, r))| *f + *s + *r > 0)
        .map(|(at, (f, s, r))| (at, f, s, r))
        .collect();

    Ok(ins)
}

fn observe(
    store: &Store,
    subject: &StableId,
    property: &str,
    value: &str,
    at: u64,
) -> Result<NodeId, StoreError> {
    store.put(&Object::Observation {
        subject: subject.clone(),
        property: property.to_string(),
        value: value.to_string(),
        source: "twin".to_string(),
        observed_at_ms: at,
    })
}

/// Write a relation unless the graph (or this run) already has it.
fn relate(
    store: &Store,
    index: &MemIndex,
    written: &mut BTreeSet<(StableId, String, StableId)>,
    from: &StableId,
    kind: &str,
    to: &StableId,
    at: u64,
) -> Result<bool, StoreError> {
    let key = (from.clone(), kind.to_string(), to.clone());
    if written.contains(&key) {
        return Ok(false);
    }
    for id in index.relations_from(from, kind) {
        if let Object::Relation { to: t, .. } = store.get(&id)? {
            if &t == to {
                written.insert(key);
                return Ok(false);
            }
        }
    }
    store.put(&Object::Relation {
        from: from.clone(),
        predicate: kind.to_string(),
        to: to.clone(),
        source: "twin".to_string(),
        observed_at_ms: at,
    })?;
    written.insert(key);
    Ok(true)
}

/// Latest observation value for (subject, property), by observation time.
pub fn latest(
    index: &MemIndex,
    store: &Store,
    subject: &StableId,
    property: &str,
) -> Result<Option<String>, StoreError> {
    Ok(latest_at(index, store, subject, property)?.map(|(_, v)| v))
}

/// Like [`latest`], but also returns when the value was observed.
pub fn latest_at(
    index: &MemIndex,
    store: &Store,
    subject: &StableId,
    property: &str,
) -> Result<Option<(u64, String)>, StoreError> {
    let mut best: Option<(u64, String)> = None;
    for id in index.observations_of(subject) {
        if let Object::Observation { property: p, value, observed_at_ms, .. } = store.get(&id)? {
            if p == property && best.as_ref().is_none_or(|(t, _)| observed_at_ms >= *t) {
                best = Some((observed_at_ms, value));
            }
        }
    }
    Ok(best)
}

/// Best-effort resolution of an import string to a twinned file path.
fn resolve_import(from_rel: &str, import: &str, files: &BTreeSet<String>) -> Option<String> {
    if files.contains(import) {
        return Some(import.to_string());
    }
    // Rust intra-crate: `crate::foo::Bar` -> <src-root>/foo.rs or foo/mod.rs,
    // where the src root is the importing file's path up through "src/".
    if let Some(rest) = import.strip_prefix("crate::") {
        let src_root = if let Some(p) = from_rel.rfind("/src/") {
            &from_rel[..p + 5]
        } else if from_rel.starts_with("src/") {
            "src/"
        } else {
            ""
        };
        if let Some(first) = rest.split("::").next() {
            for cand in [
                format!("{src_root}{first}.rs"),
                format!("{src_root}{first}/mod.rs"),
            ] {
                if files.contains(&cand) {
                    return Some(cand);
                }
            }
        }
    }
    if import.starts_with("./") || import.starts_with("../") {
        let dir = match from_rel.rsplit_once('/') {
            Some((d, _)) => d,
            None => "",
        };
        let joined = normalize(&if dir.is_empty() {
            import.to_string()
        } else {
            format!("{dir}/{import}")
        });
        for suffix in ["", ".js", ".ts", ".jsx", ".tsx", ".py", ".php", ".rs"] {
            let cand = format!("{joined}{suffix}");
            if files.contains(&cand) {
                return Some(cand);
            }
        }
        for idx in ["/index.js", "/index.ts"] {
            let cand = format!("{joined}{idx}");
            if files.contains(&cand) {
                return Some(cand);
            }
        }
    }
    None
}

/// Collapse `.` and `..` components in a relative path.
fn normalize(p: &str) -> String {
    let mut stack: Vec<&str> = Vec::new();
    for part in p.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            other => stack.push(other),
        }
    }
    stack.join("/")
}

fn git_info(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (prop, args) in [
        ("git_commit", ["rev-parse", "HEAD"]),
        ("git_branch", ["rev-parse", "--abbrev-ref"]),
    ] {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(root).args(args);
        if prop == "git_branch" {
            cmd.arg("HEAD");
        }
        if let Ok(o) = cmd.output() {
            if o.status.success() {
                let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if !v.is_empty() {
                    out.push((prop.to_string(), v));
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Notes: durable agent memory attached to any entity
// ---------------------------------------------------------------------------

pub fn add_note(store: &Store, subject: &StableId, text: &str) -> Result<NodeId, StoreError> {
    store.put(&Object::Observation {
        subject: subject.clone(),
        property: "note".to_string(),
        value: text.to_string(),
        source: "agent".to_string(),
        observed_at_ms: now_ms(),
    })
}

/// All notes on an entity, oldest first. Ordered by the event log rather
/// than by timestamp sorting: the log is chronological by construction, so
/// two notes written in the same millisecond keep their true order.
pub fn notes(
    index: &MemIndex,
    store: &Store,
    subject: &StableId,
) -> Result<Vec<(u64, String)>, StoreError> {
    let candidates: BTreeSet<NodeId> = index.observations_of(subject).into_iter().collect();
    let mut out = Vec::new();
    for id in store.put_history()? {
        if !candidates.contains(&id) {
            continue;
        }
        if let Object::Observation { property, value, observed_at_ms, .. } = store.get(&id)? {
            if property == "note" {
                out.push((observed_at_ms, value));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, tempfile::TempDir) {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::create_dir_all(src.path().join("web")).unwrap();
        fs::write(
            src.path().join("src/main.rs"),
            "use crate::util;\npub fn main() {}\nstruct Config;\n",
        )
        .unwrap();
        fs::write(src.path().join("src/util.rs"), "pub fn helper() {}\n").unwrap();
        fs::write(
            src.path().join("web/app.js"),
            "import { h } from './util';\nexport function render() {}\n",
        )
        .unwrap();
        fs::write(src.path().join("web/util.js"), "export function h() {}\n").unwrap();
        fs::write(
            src.path().join("model.php"),
            "<?php\nnamespace App;\nuse App\\Db;\nclass Model {\npublic function load() {}\n}\n",
        )
        .unwrap();
        fs::write(src.path().join("run.py"), "import os\ndef main():\n    pass\n").unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        (src, store_dir)
    }

    fn fresh_index(store: &Store) -> MemIndex {
        let mut index = MemIndex::new();
        replay(store, &mut index).unwrap();
        index
    }

    #[test]
    fn refresh_builds_structure_and_is_idempotent() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();

        let r1 = refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(r1.added.len(), 6);
        assert!(r1.symbols >= 8, "symbols across four languages: {}", r1.symbols);
        assert!(r1.relations >= 8, "contains + imports: {}", r1.relations);

        // Rust intra-crate import resolved to the file, not a module stub.
        {
            let index = fresh_index(&store);
            let main = StableId::derive(&["file", "src/main.rs"]);
            let util_rs = StableId::derive(&["file", "src/util.rs"]);
            let rels = index.relations_from(&main, "imports");
            assert_eq!(rels.len(), 1);
            match store.get(&rels[0]).unwrap() {
                Object::Relation { to, .. } => assert_eq!(to, util_rs),
                other => panic!("expected relation, got {other:?}"),
            }
        }

        let index = fresh_index(&store);
        // Structure queries: app.js contains render, imports resolved to util.js.
        let app = StableId::derive(&["file", "web/app.js"]);
        let util = StableId::derive(&["file", "web/util.js"]);
        assert_eq!(index.relations_from(&app, "contains").len(), 1);
        let imports = index.relations_from(&app, "imports");
        assert_eq!(imports.len(), 1);
        match store.get(&imports[0]).unwrap() {
            Object::Relation { to, .. } => assert_eq!(to, util, "relative import resolved to file"),
            other => panic!("expected relation, got {other:?}"),
        }
        // Unresolved imports become module entities.
        let py = StableId::derive(&["file", "run.py"]);
        let os_mod = StableId::derive(&["module", "os"]);
        let py_imports = index.relations_from(&py, "imports");
        assert_eq!(py_imports.len(), 1);
        match store.get(&py_imports[0]).unwrap() {
            Object::Relation { to, .. } => assert_eq!(to, os_mod),
            other => panic!("expected relation, got {other:?}"),
        }

        // Idempotence: an immediate second refresh writes nothing.
        let before = store.count_objects().unwrap();
        let r2 = refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(r2.unchanged, 6);
        assert!(r2.added.is_empty() && r2.changed.is_empty() && r2.deleted.is_empty());
        assert_eq!(r2.symbols + r2.relations, 0);
        assert_eq!(store.count_objects().unwrap(), before, "no graph growth");
    }

    #[test]
    fn drift_is_reported_readonly_then_recorded() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        fs::write(src.path().join("run.py"), "import sys\ndef main():\n    pass\n").unwrap();
        fs::remove_file(src.path().join("web/util.js")).unwrap();
        fs::write(src.path().join("new.rs"), "pub fn fresh() {}\n").unwrap();

        // status: reports the drift, writes nothing.
        let before = store.count_objects().unwrap();
        let s = status(&store, src.path(), "twin/app").unwrap();
        assert_eq!(s.changed, vec!["run.py".to_string()]);
        assert_eq!(s.deleted, vec!["web/util.js".to_string()]);
        assert_eq!(s.added, vec!["new.rs".to_string()]);
        assert_eq!(store.count_objects().unwrap(), before, "status is read-only");

        // refresh: records it.
        let r = refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(r.changed, vec!["run.py".to_string()]);
        assert_eq!(r.deleted, vec!["web/util.js".to_string()]);
        let index = fresh_index(&store);
        let util = StableId::derive(&["file", "web/util.js"]);
        assert_eq!(
            latest(&index, &store, &util, "present").unwrap().as_deref(),
            Some("false"),
            "deletion is an observation"
        );

        // Once recorded, the drift is gone: nothing further to report.
        let r2 = refresh(&store, src.path(), "twin/app").unwrap();
        assert!(r2.changed.is_empty() && r2.deleted.is_empty() && r2.added.is_empty());

        // The file returns: presence is restored on the next refresh.
        fs::write(src.path().join("web/util.js"), "export function h() {}\n").unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);
        assert_eq!(
            latest(&index, &store, &util, "present").unwrap().as_deref(),
            Some("true")
        );
    }

    #[test]
    fn insights_synthesize_churn_hubs_and_growth_series() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let ins = insights(&store, "twin/app").unwrap();
        assert_eq!(ins.files, 6);
        assert_eq!(ins.deleted_files, 0);
        assert!(ins.symbols >= 8);
        assert!(ins.external_modules.iter().any(|(m, _)| m == "os"));
        // app.js imports util.js; main.rs imports util.rs -> both are hubs.
        assert!(ins.hubs.iter().any(|(f, n)| f == "web/util.js" && *n == 1));
        assert!(ins.hubs.iter().any(|(f, n)| f == "src/util.rs" && *n == 1));
        assert!(ins.churn.is_empty(), "nothing edited yet");
        assert_eq!(ins.series.len(), 1, "first totals point recorded");

        // Edit a file twice across refreshes: churn appears, series grows.
        fs::write(src.path().join("run.py"), "import os\ndef main():\n    return 1\n").unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        fs::write(
            src.path().join("run.py"),
            "import os\ndef main():\n    return 2\ndef extra():\n    pass\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let ins = insights(&store, "twin/app").unwrap();
        assert!(
            ins.churn.iter().any(|(f, n)| f == "run.py" && *n == 3),
            "churn should count content versions: {:?}",
            ins.churn
        );
        assert!(ins.series.len() >= 2, "symbol growth adds a series point");

        // Idempotent refresh adds no series point.
        let points = ins.series.len();
        refresh(&store, src.path(), "twin/app").unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert_eq!(ins.series.len(), points);

        // Notes surface in insights.
        let sid = StableId::derive(&["file", "run.py"]);
        add_note(&store, &sid, "agent: rewrote main twice while iterating").unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert!(ins.notes.iter().any(|(_, e, t)| e == "run.py" && t.contains("rewrote")));
    }

    #[test]
    fn notes_attach_to_entities_and_survive_in_order() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let sid = StableId::derive(&["file", "src/main.rs"]);
        add_note(&store, &sid, "entry point; config loading lives here").unwrap();
        add_note(&store, &sid, "Config struct is a stub").unwrap();

        let index = fresh_index(&store);
        let found = notes(&index, &store, &sid).unwrap();
        assert_eq!(found.len(), 2);
        assert!(found[0].1.contains("entry point"));
        assert!(found[1].1.contains("stub"));
    }
}
