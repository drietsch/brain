//! The twin's own tests.

use super::*;
use crate::agents::{self, AgentDoc};
use crate::docs::{self, DocMeta};
use crate::symbols;
use crate::testing;
use brain_core::ids::{NodeId, StableId};
use brain_core::object::Object;
use brain_index::{replay, Index, MemIndex};
use brain_store::{now_ms, Store, StoreError};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;

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
        fs::write(
            src.path().join("run.py"),
            "import os\ndef main():\n    pass\n",
        )
        .unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        (src, store_dir)
    }

    fn fresh_index(store: &Store) -> MemIndex {
        let mut index = MemIndex::new();
        replay(store, &mut index).unwrap();
        index
    }

    #[test]
    fn re_adding_a_done_plan_reactivates_it() {
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        let meta = crate::docs::parse_content(
            crate::docs::DocKind::Plan,
            "sprint",
            "# Sprint\n\nv1.\n",
            None,
            None,
        );
        add_doc(&store, "twin/app", &meta, "# Sprint\n\nv1.\n", "test").unwrap();

        let sid = StableId::derive(&["plan", "twin/app", "sprint"]);
        let index = fresh_index(&store);
        crate::lifecycle::set(&store, &index, &sid, crate::lifecycle::Lifecycle::Done, None)
            .unwrap();

        // Re-registering with new content is a statement of intent: the
        // plan is being worked again, so it returns to the active lists.
        let meta = crate::docs::parse_content(
            crate::docs::DocKind::Plan,
            "sprint",
            "# Sprint\n\nv2 — reopened.\n",
            None,
            None,
        );
        let out = add_doc(&store, "twin/app", &meta, "# Sprint\n\nv2 — reopened.\n", "test")
            .unwrap();
        assert!(out.wrote);
        let index = fresh_index(&store);
        let (state, _) = crate::lifecycle::of(&index, &store, &sid).unwrap();
        assert_eq!(state, crate::lifecycle::Lifecycle::Active);
    }

    #[test]
    fn refresh_builds_structure_and_is_idempotent() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();

        let r1 = refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(r1.added.len(), 6);
        assert!(
            r1.symbols >= 8,
            "symbols across four languages: {}",
            r1.symbols
        );
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

        fs::write(
            src.path().join("run.py"),
            "import sys\ndef main():\n    pass\n",
        )
        .unwrap();
        fs::remove_file(src.path().join("web/util.js")).unwrap();
        fs::write(src.path().join("new.rs"), "pub fn fresh() {}\n").unwrap();

        // status: reports the drift, writes nothing.
        let before = store.count_objects().unwrap();
        let s = status(&store, src.path(), "twin/app").unwrap();
        assert_eq!(s.changed, vec!["run.py".to_string()]);
        assert_eq!(s.deleted, vec!["web/util.js".to_string()]);
        assert_eq!(s.added, vec!["new.rs".to_string()]);
        assert_eq!(
            store.count_objects().unwrap(),
            before,
            "status is read-only"
        );

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
        fs::write(
            src.path().join("run.py"),
            "import os\ndef main():\n    return 1\n",
        )
        .unwrap();
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
        assert!(ins
            .notes
            .iter()
            .any(|(_, e, t)| e == "run.py" && t.contains("rewrote")));
    }

    /// Every spelling of a moment resolves, and a spelling that means
    /// nothing gets a sentence naming the accepted forms.
    #[test]
    fn resolve_when_reads_every_spelling_of_a_moment() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);

        assert_eq!(
            resolve_when(&store, &index, "twin/app", "live").unwrap(),
            u64::MAX
        );
        assert_eq!(
            resolve_when(&store, &index, "twin/app", "1785400000000").unwrap(),
            1785400000000
        );
        let five_min = resolve_when(&store, &index, "twin/app", "5m").unwrap();
        assert!(five_min <= now_ms().saturating_sub(5 * 60 * 1000) + 1000);

        // A baseline name resolves to the moment it names.
        crate::baseline::add(&store, &index, "twin/app", "mark", 123_456_789_012).unwrap();
        let index = fresh_index(&store);
        assert_eq!(
            resolve_when(&store, &index, "twin/app", "mark").unwrap(),
            123_456_789_012
        );

        // A commit prefix resolves to when the twin saw it as HEAD.
        let repo = StableId::derive(&["repo", "twin/app"]);
        observe_src(&store, &repo, "git_commit", "abcdef1234567890", "twin", 424_242).unwrap();
        let index = fresh_index(&store);
        assert_eq!(
            resolve_when(&store, &index, "twin/app", "abcdef12").unwrap(),
            424_242
        );

        let err = resolve_when(&store, &index, "twin/app", "nonsense").unwrap_err();
        assert!(err.contains("cannot resolve"), "{err}");
    }

    #[test]
    fn quality_series_appends_on_change_and_holds_when_idle() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        // The first refresh takes a complete baseline reading.
        let ins = insights(&store, "twin/app").unwrap();
        assert_eq!(ins.quality.len(), 1, "baseline reading recorded");
        let q = &ins.quality[0];
        assert_eq!(q.tests, None, "no run imported yet");
        assert_eq!((q.features_done, q.features_total), (0, 0));
        assert_eq!(q.uncorroborated, 0);

        // A test run moves the picture; the next refresh takes a reading.
        std::thread::sleep(std::time::Duration::from_millis(5));
        let run = "test calc::tests::t_add ... ok\ntest web::render ... FAILED\n";
        testing::record_run(&store, "twin/app", &testing::parse_report(run), run).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert_eq!(ins.quality.len(), 2, "a moved number appends a reading");
        let q = ins.quality.last().unwrap();
        assert_eq!(q.tests, Some((1, 2)), "one of two tests passing");

        // Idempotent refresh appends nothing to either series.
        let readings = ins.quality.len();
        let growth = ins.series.len();
        refresh(&store, src.path(), "twin/app").unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert_eq!(ins.quality.len(), readings);
        assert_eq!(ins.series.len(), growth);

        // A growth-only change moves the growth series and not the
        // quality series — the two are guarded independently.
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(src.path().join("more.py"), "def more():\n    pass\n").unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert!(ins.series.len() > growth, "growth series moved");
        assert_eq!(ins.quality.len(), readings, "quality series held");
    }

    #[test]
    fn decisions_and_plans_are_captured_and_linked() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        fs::create_dir_all(src.path().join("docs/adr")).unwrap();
        fs::create_dir_all(src.path().join("docs/plans")).unwrap();
        fs::write(
            src.path().join("docs/adr/adr-001-storage.md"),
            "# Use content addressing\n\nStatus: proposed\n\nAffects src/main.rs directly.\n",
        )
        .unwrap();
        fs::write(
            src.path().join("docs/plans/plan-v1.md"),
            "# Plan v1\n\nRefactor src/util.rs and web/app.js.\n",
        )
        .unwrap();

        let r = refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(r.docs.len(), 2, "both documents captured: {:?}", r.docs);

        let index = fresh_index(&store);
        let adr = StableId::derive(&["decision", "twin/app", "adr-001-storage"]);
        let plan = StableId::derive(&["plan", "twin/app", "plan-v1"]);
        assert_eq!(
            latest(&index, &store, &adr, "status").unwrap().as_deref(),
            Some("proposed")
        );
        assert_eq!(
            latest(&index, &store, &adr, "title").unwrap().as_deref(),
            Some("Use content addressing")
        );
        assert!(latest(&index, &store, &plan, "content")
            .unwrap()
            .unwrap()
            .contains("Refactor"));

        // Linked: mentions -> the file it names, concerns -> repo,
        // recorded_in -> the markdown file entity.
        let main_sid = StableId::derive(&["file", "src/main.rs"]);
        let rels = index.relations_from(&adr, "mentions");
        assert_eq!(rels.len(), 1);
        match store.get(&rels[0]).unwrap() {
            Object::Relation { to, .. } => assert_eq!(to, main_sid),
            other => panic!("expected relation, got {other:?}"),
        }
        assert_eq!(index.relations_from(&adr, "concerns").len(), 1);
        assert_eq!(index.relations_from(&adr, "recorded_in").len(), 1);
        assert_eq!(index.relations_from(&plan, "mentions").len(), 2);

        // Idempotence: an immediate second refresh writes nothing.
        let before = store.count_objects().unwrap();
        let r2 = refresh(&store, src.path(), "twin/app").unwrap();
        assert!(r2.docs.is_empty());
        assert_eq!(store.count_objects().unwrap(), before, "no graph growth");

        // A status change is a new observation: decisions get a timeline.
        fs::write(
            src.path().join("docs/adr/adr-001-storage.md"),
            "# Use content addressing\n\nStatus: accepted\n\nAffects src/main.rs directly.\n",
        )
        .unwrap();
        // And a superseding decision links to what it replaces.
        fs::write(
            src.path().join("docs/adr/adr-002-sync.md"),
            "# Sync differently\n\nStatus: proposed\nSupersedes: adr-001-storage.md\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);
        assert_eq!(
            latest(&index, &store, &adr, "status").unwrap().as_deref(),
            Some("accepted")
        );
        let statuses = index
            .observations_of(&adr)
            .iter()
            .filter_map(|id| store.get(id).ok())
            .filter(|o| matches!(o, Object::Observation { property, .. } if property == "status"))
            .count();
        assert_eq!(statuses, 2, "status history is a timeline");
        let adr2 = StableId::derive(&["decision", "twin/app", "adr-002-sync"]);
        let sup = index.relations_from(&adr2, "supersedes");
        assert_eq!(sup.len(), 1);
        match store.get(&sup[0]).unwrap() {
            Object::Relation { to, .. } => assert_eq!(to, adr),
            other => panic!("expected relation, got {other:?}"),
        }

        // Insights surface only the living decision set: the superseded ADR
        // is history, and files it alone mentioned lose their decided tag.
        let ins = insights(&store, "twin/app").unwrap();
        assert!(
            !ins.decisions.iter().any(|(s, _, _)| s == "adr-001-storage"),
            "superseded decisions leave the list: {:?}",
            ins.decisions
        );
        assert!(ins
            .decisions
            .iter()
            .any(|(s, _, st)| s == "adr-002-sync" && st == "proposed"));
        assert!(ins.plans.iter().any(|(s, _)| s == "plan-v1"));
        assert!(
            !ins.decided.contains("src/main.rs"),
            "its rationale was superseded"
        );
        assert!(!ins.decided.contains("run.py"));
        let (lc, why) = crate::lifecycle::of(&index, &store, &adr).unwrap();
        assert_eq!(lc, crate::lifecycle::Lifecycle::Superseded);
        assert!(why.contains("adr-002-sync"), "{why}");
    }

    #[test]
    fn skills_and_agent_config_are_captured() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        fs::create_dir_all(src.path().join(".claude/skills/deploy")).unwrap();
        fs::create_dir_all(src.path().join(".claude/agents")).unwrap();
        fs::write(
            src.path().join(".claude/skills/deploy/SKILL.md"),
            "---\nname: deploy\ndescription: Ship src/main.rs safely\n---\n\n# Deploy\n",
        )
        .unwrap();
        fs::write(
            src.path().join("CLAUDE.md"),
            "# Project rules\n\nStart at src/main.rs.\n",
        )
        .unwrap();
        fs::write(
            src.path().join(".claude/agents/reviewer.md"),
            "---\nname: reviewer\ndescription: Reviews diffs\n---\nReview carefully.\n",
        )
        .unwrap();
        fs::write(src.path().join(".cursorrules"), "Prefer small functions.\n").unwrap();

        let r = refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(r.docs.len(), 4, "all agent docs captured: {:?}", r.docs);

        let index = fresh_index(&store);
        let skill = StableId::derive(&["skill", "twin/app", "deploy"]);
        assert_eq!(
            latest(&index, &store, &skill, "description")
                .unwrap()
                .as_deref(),
            Some("Ship src/main.rs safely")
        );
        assert_eq!(
            latest(&index, &store, &skill, "agent").unwrap().as_deref(),
            Some("claude")
        );
        // The skill mentions src/main.rs (from its description in content).
        let main_sid = StableId::derive(&["file", "src/main.rs"]);
        let mentioned: Vec<_> = index
            .relations_from(&skill, "mentions")
            .iter()
            .filter_map(|id| match store.get(id) {
                Ok(Object::Relation { to, .. }) => Some(to),
                _ => None,
            })
            .collect();
        assert_eq!(mentioned, vec![main_sid]);

        let claude_md = StableId::derive(&["agent_config", "twin/app", "claude.md"]);
        assert_eq!(
            latest(&index, &store, &claude_md, "role")
                .unwrap()
                .as_deref(),
            Some("instructions")
        );
        let reviewer = StableId::derive(&["agent_config", "twin/app", "reviewer"]);
        assert_eq!(
            latest(&index, &store, &reviewer, "role")
                .unwrap()
                .as_deref(),
            Some("subagent")
        );
        let cursor = StableId::derive(&["agent_config", "twin/app", ".cursorrules"]);
        assert_eq!(
            latest(&index, &store, &cursor, "agent").unwrap().as_deref(),
            Some("cursor")
        );

        // Idempotence still holds with agent docs present.
        let before = store.count_objects().unwrap();
        let r2 = refresh(&store, src.path(), "twin/app").unwrap();
        assert!(r2.docs.is_empty());
        assert_eq!(store.count_objects().unwrap(), before, "no graph growth");

        // Insights surface them.
        let ins = insights(&store, "twin/app").unwrap();
        assert!(ins
            .skills
            .iter()
            .any(|(s, a, d)| s == "deploy" && a == "claude" && d.contains("Ship")));
        assert!(ins
            .agent_configs
            .iter()
            .any(|(s, _, r)| s == "claude.md" && r == "instructions"));
        assert!(ins
            .agent_configs
            .iter()
            .any(|(s, _, r)| s == "reviewer" && r == "subagent"));

        // Explicit add for an out-of-repo skill (user-level ~/.claude).
        let content = "---\nname: triage\ndescription: Sort issues\n---\nSteps.\n";
        let doc = agents::parse_agent_doc("home/.claude/skills/triage/SKILL.md", content).unwrap();
        let out = add_agent_doc(&store, "twin/app", &doc, content, "claude-code").unwrap();
        assert!(out.wrote);
        let again = add_agent_doc(&store, "twin/app", &doc, content, "claude-code").unwrap();
        assert!(
            !again.wrote,
            "explicit re-add of unchanged skill writes nothing"
        );
        let ins = insights(&store, "twin/app").unwrap();
        assert!(ins.skills.iter().any(|(s, _, _)| s == "triage"));
    }

    #[test]
    fn explicit_add_doc_records_out_of_repo_plans() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        // A plan file living outside the observed tree (e.g. ~/.claude/plans).
        let content = "# The session plan\n\nTouch src/main.rs and run.py.\n";
        let meta = docs::parse_content(docs::DocKind::Plan, "session-plan", content, None, None);
        let out = add_doc(&store, "twin/app", &meta, content, "claude-code").unwrap();
        assert!(out.wrote);
        assert_eq!(
            out.mentions,
            vec!["run.py".to_string(), "src/main.rs".to_string()]
        );

        // Re-adding the identical document writes nothing.
        let before = store.count_objects().unwrap();
        let again = add_doc(&store, "twin/app", &meta, content, "claude-code").unwrap();
        assert!(!again.wrote);
        assert_eq!(store.count_objects().unwrap(), before);

        // The observations carry the explicit source, and insights list it.
        let index = fresh_index(&store);
        assert!(index.observations_of(&out.sid).iter().any(|id| matches!(
            store.get(id).unwrap(),
            Object::Observation { source, .. } if source == "claude-code"
        )));
        let ins = insights(&store, "twin/app").unwrap();
        assert!(ins
            .plans
            .iter()
            .any(|(s, t)| s == "session-plan" && t == "The session plan"));
    }

    #[test]
    fn templates_record_conformance_and_features_evaluate_done() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        crate::templates::seed(&store).unwrap();
        fs::create_dir_all(src.path().join("docs/adr")).unwrap();
        // One ADR honors the contract, one is missing its status.
        fs::write(
            src.path().join("docs/adr/adr-001-good.md"),
            "# Good decision\n\nStatus: accepted\n\nBecause src/main.rs needed it.\n",
        )
        .unwrap();
        fs::write(
            src.path().join("docs/adr/adr-002-bare.md"),
            "prose without contract\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let index = fresh_index(&store);
        let good = StableId::derive(&["decision", "twin/app", "adr-001-good"]);
        let bare = StableId::derive(&["decision", "twin/app", "adr-002-bare"]);
        assert_eq!(
            latest(&index, &store, &good, "conforms")
                .unwrap()
                .as_deref(),
            Some("true")
        );
        assert_eq!(
            latest(&index, &store, &bare, "conforms")
                .unwrap()
                .as_deref(),
            Some("false")
        );
        assert_eq!(
            latest(&index, &store, &bare, "missing").unwrap().as_deref(),
            Some("title,status")
        );
        assert_eq!(index.relations_from(&good, "conforms_to").len(), 1);

        // Insights surface the violation; fixing the file clears it.
        let ins = insights(&store, "twin/app").unwrap();
        assert!(ins
            .nonconforming
            .iter()
            .any(|(s, k, m)| { s == "adr-002-bare" && k == "decision" && m.contains("status") }));
        fs::write(
            src.path().join("docs/adr/adr-002-bare.md"),
            "# Now titled\n\nStatus: proposed\n\nprose with contract\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);
        assert_eq!(
            latest(&index, &store, &bare, "conforms")
                .unwrap()
                .as_deref(),
            Some("true")
        );
        let ins = insights(&store, "twin/app").unwrap();
        assert!(ins.nonconforming.is_empty());

        // Refresh stays idempotent with templates seeded.
        let before = store.count_objects().unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(store.count_objects().unwrap(), before, "no graph growth");

        // Feature registry: register, link, evaluate done against the DoD.
        let (fsid, wrote) =
            crate::features::add(&store, "twin/app", "render", "Rendering", "building").unwrap();
        assert!(wrote);
        let index = fresh_index(&store);
        let (main_sid, kind) =
            crate::features::resolve_target(&store, &index, "twin/app", "src/main.rs")
                .unwrap()
                .unwrap();
        assert_eq!(kind, "file");
        crate::features::link(&store, "twin/app", "render", "implemented_by", &main_sid).unwrap();
        let (adr_sid, kind) =
            crate::features::resolve_target(&store, &index, "twin/app", "adr-001-good")
                .unwrap()
                .unwrap();
        assert_eq!(kind, "decision");
        crate::features::link(&store, "twin/app", "render", "decided_by", &adr_sid).unwrap();

        let index = fresh_index(&store);
        let report = crate::features::evaluate(&store, &index, "twin/app", "render").unwrap();
        assert!(!report.done, "2 of 4 DoD predicates met");
        assert_eq!(
            report.checks.len(),
            4,
            "DoD comes from the seeded feature template"
        );
        assert_eq!(report.checks.iter().filter(|c| c.count > 0).count(), 2);
        assert!(
            crate::features::record_done(&store, &index, "twin/app", "render", &report).unwrap()
        );
        let index = fresh_index(&store);
        assert!(
            !crate::features::record_done(&store, &index, "twin/app", "render", &report).unwrap(),
            "unchanged done state writes nothing"
        );
        assert_eq!(
            latest(&index, &store, &fsid, "done").unwrap().as_deref(),
            Some("false")
        );

        // Complete the DoD: the feature flips to done.
        let test_sid = StableId::derive(&["file", "run.py"]);
        crate::features::link(&store, "twin/app", "render", "tested_by", &test_sid).unwrap();
        let readme = StableId::derive(&["file", "web/app.js"]);
        crate::features::link(&store, "twin/app", "render", "documented_in", &readme).unwrap();
        let index = fresh_index(&store);
        let report = crate::features::evaluate(&store, &index, "twin/app", "render").unwrap();
        assert!(report.done);

        // Insights render the matrix fraction.
        let ins = insights(&store, "twin/app").unwrap();
        assert!(ins
            .features
            .iter()
            .any(|f| f.slug == "render" && f.status == "building" && f.fraction == "4/4"));

        // A parent is judged by its parts (ADR-028), and insights must say
        // so too. Reading the fraction off the parent's own links made the
        // root of a spine report what it happened to be linked to directly
        // while every part under it was ready.
        crate::features::add(&store, "twin/app", "surface", "Surface", "building").unwrap();
        let parent = crate::features::feature_sid("twin/app", "surface");
        crate::features::link(&store, "twin/app", "render", "part_of", &parent).unwrap();
        let index = fresh_index(&store);
        let report = crate::features::evaluate(&store, &index, "twin/app", "surface").unwrap();
        assert!(report.by_parts() && report.done, "its one part is ready");
        assert_eq!(report.checks.iter().filter(|c| c.count > 0).count(), 0);

        let ins = insights(&store, "twin/app").unwrap();
        let surface = ins.features.iter().find(|f| f.slug == "surface").unwrap();
        assert!(surface.by_parts);
        assert!(surface.done, "the part is ready, so the parent is");
        assert_eq!(
            surface.fraction, "1/1",
            "the fraction counts parts, not the parent's own links"
        );
    }

    #[test]
    fn tests_classify_cover_and_protocols_form_timelines() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        fs::write(
            src.path().join("web/app.test.js"),
            "import { render } from './app';\ntest('renders', () => {});\nit('updates', () => {});\n",
        )
        .unwrap();
        fs::write(
            src.path().join("src/calc.rs"),
            "pub fn add(a: i64, b: i64) -> i64 { a + b }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t_add() {}\n}\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let index = fresh_index(&store);
        let spec = StableId::derive(&["file", "web/app.test.js"]);
        let app = StableId::derive(&["file", "web/app.js"]);
        let calc = StableId::derive(&["file", "src/calc.rs"]);
        assert_eq!(
            latest(&index, &store, &spec, "test_framework")
                .unwrap()
                .as_deref(),
            Some("jest")
        );
        assert_eq!(
            latest(&index, &store, &spec, "tests_declared")
                .unwrap()
                .as_deref(),
            Some("2")
        );
        assert_eq!(
            latest(&index, &store, &spec, "file_role")
                .unwrap()
                .as_deref(),
            Some("test")
        );
        // The spec covers the file it imports; inline Rust tests classify
        // the file without marking it role=test.
        assert_eq!(index.relations_to(&app, "covers").len(), 1);
        assert_eq!(
            latest(&index, &store, &calc, "test_framework")
                .unwrap()
                .as_deref(),
            Some("rust")
        );
        assert_eq!(latest(&index, &store, &calc, "file_role").unwrap(), None);

        // Refresh stays idempotent with test classification present.
        let before = store.count_objects().unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(store.count_objects().unwrap(), before, "no graph growth");

        // Protocol 1: a cargo run with one failure.
        let run1 = "test calc::tests::t_add ... ok\ntest web::render ... FAILED\n";
        let report = testing::parse_report(run1);
        let out = testing::record_run(&store, "twin/app", &report, run1).unwrap();
        assert!(out.wrote);
        assert_eq!((out.total, out.passed, out.failed), (2, 1, 1));
        assert_eq!(out.failing, vec!["web::render".to_string()]);
        assert_eq!(out.transitions, 0, "first observations are not transitions");

        // Re-importing the identical report writes nothing.
        let before = store.count_objects().unwrap();
        let again = testing::record_run(&store, "twin/app", &report, run1).unwrap();
        assert!(!again.wrote);
        assert_eq!(store.count_objects().unwrap(), before);

        // The failing case is queryable, and the run left Behavioral
        // evidence on the repo entity.
        let index = fresh_index(&store);
        assert_eq!(
            testing::failing_cases(&store, &index, "twin/app").unwrap(),
            vec!["web::render".to_string()]
        );
        let repo_sid = StableId::derive(&["repo", "twin/app"]);
        let repo_node = index.entity_nodes(&repo_sid)[0];
        let evidence = index.evidence_for(&repo_node);
        assert_eq!(evidence.len(), 1);
        match store.get(&evidence[0]).unwrap() {
            Object::Evidence { passed, level, .. } => {
                assert!(!passed);
                assert_eq!(level, brain_core::object::VerificationLevel::Behavioral);
            }
            other => panic!("expected evidence, got {other:?}"),
        }

        // Protocol 2: the failure is fixed — a pass->fail->pass timeline.
        let run2 = "test calc::tests::t_add ... ok\ntest web::render ... ok\n";
        let out =
            testing::record_run(&store, "twin/app", &testing::parse_report(run2), run2).unwrap();
        assert!(out.wrote);
        assert_eq!(out.transitions, 1, "fail -> pass is a recorded transition");
        let index = fresh_index(&store);
        assert!(testing::failing_cases(&store, &index, "twin/app")
            .unwrap()
            .is_empty());
        assert_eq!(testing::runs(&store, &index, "twin/app").unwrap().len(), 2);

        // A JUnit (Playwright-style) run links cases to their spec file.
        let junit = "<testsuite>\n  <testcase classname=\"web/app.test.js\" name=\"renders\"/>\n</testsuite>\n";
        testing::record_run(&store, "twin/app", &testing::parse_report(junit), junit).unwrap();
        let index = fresh_index(&store);
        let case = StableId::derive(&["test", "twin/app", "web/app.test.js::renders"]);
        assert_eq!(index.relations_from(&case, "defined_in").len(), 1);

        // Insights: totals, last run, and the untested hub (src/util.rs is
        // imported but has no tests and no covering spec; web/app.js is
        // covered by the spec).
        let ins = insights(&store, "twin/app").unwrap();
        assert_eq!(ins.test_files, 1);
        assert!(ins.tests_declared >= 3);
        let (_, total, passed, failed) = ins.last_run.unwrap();
        assert_eq!((total, passed, failed), (1, 1, 0));
        assert!(ins.failing.is_empty());
        assert!(ins.untested_hubs.iter().any(|(f, _)| f == "src/util.rs"));
        assert!(!ins.untested_hubs.iter().any(|(f, _)| f == "web/app.js"));
    }

    #[test]
    fn docs_go_stale_when_mentioned_files_change_after_them() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        fs::create_dir_all(src.path().join("docs/adr")).unwrap();
        fs::write(
            src.path().join("docs/adr/adr-001-main.md"),
            "# Main design\n\nStatus: accepted\n\nAll logic lives in src/main.rs.\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert!(
            ins.stale_docs.is_empty(),
            "freshly captured doc is not stale"
        );

        // The mentioned file changes after the doc was recorded.
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(
            src.path().join("src/main.rs"),
            "use crate::util;\npub fn main() { util::helper() }\nstruct Config;\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert_eq!(ins.stale_docs.len(), 1);
        let d = &ins.stale_docs[0];
        assert_eq!(d.slug, "adr-001-main");
        assert_eq!(d.kind, "decision");
        assert_eq!(
            d.severity,
            Severity::Info,
            "decisions are records: info by default"
        );
        assert_eq!(d.changed, vec!["src/main.rs".to_string()]);

        // Acknowledging resets the clock without touching the file.
        let adr = StableId::derive(&["decision", "twin/app", "adr-001-main"]);
        std::thread::sleep(std::time::Duration::from_millis(5));
        ack(&store, &adr, "checked against current code").unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert!(
            ins.stale_docs.is_empty(),
            "acknowledged doc is fresh, file untouched"
        );

        // A later change makes it stale again; updating the doc clears it.
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(
            src.path().join("src/main.rs"),
            "use crate::util;\npub fn main() { util::helper() }\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(insights(&store, "twin/app").unwrap().stale_docs.len(), 1);
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(
            src.path().join("docs/adr/adr-001-main.md"),
            "# Main design\n\nStatus: accepted\n\nAll logic lives in src/main.rs; helper moved in.\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert!(ins.stale_docs.is_empty(), "re-touched doc is fresh again");

        // A done plan never rots: give it a mention, finish it, churn away.
        fs::create_dir_all(src.path().join("docs/plans")).unwrap();
        fs::write(
            src.path().join("docs/plans/refactor.md"),
            "# Refactor\n\nRework src/main.rs.\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let plan = StableId::derive(&["plan", "twin/app", "refactor"]);
        {
            let index = fresh_index(&store);
            crate::lifecycle::set(
                &store,
                &index,
                &plan,
                crate::lifecycle::Lifecycle::Done,
                None,
            )
            .unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(
            src.path().join("src/main.rs"),
            "pub fn main() { /* rewritten */ }\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert!(
            !ins.stale_docs.iter().any(|d| d.slug == "refactor"),
            "a finished plan is history, not rot: {:?}",
            ins.stale_docs
        );

        // rot=none on the kind's template exempts it entirely.
        crate::templates::seed(&store).unwrap();
        let tmpl = crate::templates::template_sid("adr");
        std::thread::sleep(std::time::Duration::from_millis(2));
        observe_src(&store, &tmpl, "rot", "none", "agent", now_ms()).unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert!(
            !ins.stale_docs.iter().any(|d| d.kind == "decision"),
            "rot=none exempts the kind: {:?}",
            ins.stale_docs
        );
    }

    #[test]
    fn graph_defined_capture_rules_teach_new_artifact_kinds() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        crate::templates::seed(&store).unwrap();

        // Teach the store a "runbook" kind purely with observations.
        let tmpl = crate::templates::template_sid("runbook");
        store
            .put(&Object::Entity {
                id: tmpl.clone(),
                entity_kind: "template".to_string(),
                labels: BTreeMap::new(),
            })
            .unwrap();
        let now = now_ms();
        for (prop, value) in [
            ("applies_to", "runbook"),
            ("capture", "docs/runbooks/*.md"),
            ("fields", "title=heading, service=line"),
            ("requires", "title,service"),
        ] {
            observe_src(&store, &tmpl, prop, value, "agent", now).unwrap();
        }

        fs::create_dir_all(src.path().join("docs/runbooks")).unwrap();
        fs::write(
            src.path().join("docs/runbooks/deploy.md"),
            "# Deploy safely\n\nService: checkout\n\nRestart src/main.rs afterwards.\n",
        )
        .unwrap();
        fs::write(
            src.path().join("docs/runbooks/rollback.md"),
            "just some prose\n",
        )
        .unwrap();

        let r = refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(r.docs.len(), 2, "both runbooks captured: {:?}", r.docs);

        let index = fresh_index(&store);
        let deploy = StableId::derive(&["runbook", "twin/app", "deploy"]);
        assert_eq!(
            latest(&index, &store, &deploy, "title").unwrap().as_deref(),
            Some("Deploy safely")
        );
        assert_eq!(
            latest(&index, &store, &deploy, "service")
                .unwrap()
                .as_deref(),
            Some("checkout"),
            "extracted field became an observation"
        );
        assert_eq!(
            latest(&index, &store, &deploy, "conforms")
                .unwrap()
                .as_deref(),
            Some("true")
        );
        // Mentions and concerns come from the shared core.
        let main_sid = StableId::derive(&["file", "src/main.rs"]);
        let mentioned: Vec<_> = index
            .relations_from(&deploy, "mentions")
            .iter()
            .filter_map(|id| match store.get(id) {
                Ok(Object::Relation { to, .. }) => Some(to),
                _ => None,
            })
            .collect();
        assert_eq!(mentioned, vec![main_sid.clone()]);
        assert_eq!(index.relations_from(&deploy, "concerns").len(), 1);

        // The prose-only runbook fails its contract — recorded, not rejected.
        let rollback = StableId::derive(&["runbook", "twin/app", "rollback"]);
        assert_eq!(
            latest(&index, &store, &rollback, "conforms")
                .unwrap()
                .as_deref(),
            Some("false")
        );
        let ins = insights(&store, "twin/app").unwrap();
        assert!(ins
            .nonconforming
            .iter()
            .any(|(s, k, m)| { s == "rollback" && k == "runbook" && m.contains("service") }));
        assert!(ins
            .custom_artifacts
            .iter()
            .any(|(k, n)| k == "runbook" && *n == 2));

        // Idempotence holds for rule-captured artifacts too.
        let before = store.count_objects().unwrap();
        let r2 = refresh(&store, src.path(), "twin/app").unwrap();
        assert!(r2.docs.is_empty());
        assert_eq!(store.count_objects().unwrap(), before, "no graph growth");

        // Staleness applies to the custom kind: the mentioned file changes.
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(
            src.path().join("src/main.rs"),
            "use crate::util;\npub fn main() { /* changed */ }\nstruct Config;\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert!(ins.stale_docs.iter().any(|d| {
            d.slug == "deploy"
                && d.kind == "runbook"
                && d.severity == Severity::Warn
                && d.changed.contains(&"src/main.rs".to_string())
        }));
    }

    #[test]
    fn rust_cross_crate_imports_resolve_to_sibling_crates() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("crates/core-lib/src")).unwrap();
        fs::create_dir_all(src.path().join("crates/app/src")).unwrap();
        fs::write(
            src.path().join("crates/core-lib/src/lib.rs"),
            "pub mod ids;\n",
        )
        .unwrap();
        fs::write(
            src.path().join("crates/core-lib/src/ids.rs"),
            "pub struct Id;\n",
        )
        .unwrap();
        fs::write(
            src.path().join("crates/app/src/lib.rs"),
            "use core_lib::ids::Id;\nuse core_lib::helper;\npub fn go() {}\n",
        )
        .unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/ws").unwrap();

        let index = fresh_index(&store);
        let app = StableId::derive(&["file", "crates/app/src/lib.rs"]);
        let ids_rs = StableId::derive(&["file", "crates/core-lib/src/ids.rs"]);
        let core_root = StableId::derive(&["file", "crates/core-lib/src/lib.rs"]);
        let targets: Vec<_> = index
            .relations_from(&app, "imports")
            .iter()
            .filter_map(|id| match store.get(id) {
                Ok(Object::Relation { to, .. }) => Some(to),
                _ => None,
            })
            .collect();
        // `core_lib::ids::Id` -> the module file (hyphens matched from
        // underscores); `core_lib::helper` -> the crate root fallback.
        assert!(targets.contains(&ids_rs), "{targets:?}");
        assert!(targets.contains(&core_root), "{targets:?}");

        // A --full refresh after an extractor upgrade is guarded too:
        // reprocessing everything writes no duplicate facts.
        let before = store.count_objects().unwrap();
        refresh_full(&store, src.path(), "twin/ws").unwrap();
        assert_eq!(
            store.count_objects().unwrap(),
            before,
            "full reprocess, zero growth"
        );
    }

    #[test]
    fn files_at_reads_the_twin_as_it_was() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);
        let py = StableId::derive(&["file", "run.py"]);
        let (t1, old_hash) = latest_at(&index, &store, &py, "content_b3")
            .unwrap()
            .expect("first hash");

        // Later: run.py changes, new.rs appears.
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(
            src.path().join("run.py"),
            "import os\ndef main():\n    return 9\n",
        )
        .unwrap();
        fs::write(src.path().join("new.rs"), "pub fn fresh() {}\n").unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let index = fresh_index(&store);
        // As of t1: the old hash, and new.rs does not exist yet.
        let then = files_at(&store, &index, "twin/app", t1).unwrap();
        let at_t1 = then
            .iter()
            .find(|(r, _)| r == "run.py")
            .expect("run.py existed");
        assert_eq!(at_t1.1, old_hash);
        assert!(!then.iter().any(|(r, _)| r == "new.rs"));
        // Now: the new hash, and new.rs is present.
        let now = files_at(&store, &index, "twin/app", u64::MAX).unwrap();
        let current = now.iter().find(|(r, _)| r == "run.py").unwrap();
        assert_ne!(current.1, old_hash);
        assert!(now.iter().any(|(r, _)| r == "new.rs"));
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

    #[test]
    fn vanished_structure_is_retracted_and_restored_edges_reuse_relations() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::write(
            src.path().join("src/main.rs"),
            "use crate::util;\npub fn main() {}\npub fn extra() {}\n",
        )
        .unwrap();
        fs::write(src.path().join("src/util.rs"), "pub fn helper() {}\n").unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let main = StableId::derive(&["file", "src/main.rs"]);
        let util = StableId::derive(&["file", "src/util.rs"]);
        {
            let index = fresh_index(&store);
            assert_eq!(
                live_from(&index, &store, &main, "contains").unwrap().len(),
                2
            );
            assert_eq!(live_to(&index, &store, &util, "imports").unwrap().len(), 1);
        }

        // Drop one symbol and the import: the vanished structure is retracted.
        std::thread::sleep(std::time::Duration::from_millis(3));
        fs::write(src.path().join("src/main.rs"), "pub fn main() {}\n").unwrap();
        let r = refresh(&store, src.path(), "twin/app").unwrap();
        assert!(
            r.retracted >= 2,
            "symbol + import retracted: {}",
            r.retracted
        );
        let index = fresh_index(&store);
        assert_eq!(
            live_from(&index, &store, &main, "contains").unwrap().len(),
            1
        );
        assert!(live_to(&index, &store, &util, "imports")
            .unwrap()
            .is_empty());
        // The relation objects themselves remain — history is never destroyed.
        assert_eq!(index.relations_from(&main, "contains").len(), 2);
        // Insights count live structure only.
        let ins = insights_with(&store, &index, "twin/app").unwrap();
        assert_eq!(ins.symbols, 2, "main() + helper(), extra() gone");
        assert!(
            ins.hubs.is_empty(),
            "util.rs stopped being a hub: {:?}",
            ins.hubs
        );

        // Idempotence: retraction is a transition, not a repeated write.
        let before = store.count_objects().unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(
            store.count_objects().unwrap(),
            before,
            "no growth on re-refresh"
        );

        // Re-adding the import restores the edge via the existing relation
        // object: one active=true observation, no duplicate relation.
        std::thread::sleep(std::time::Duration::from_millis(3));
        fs::write(
            src.path().join("src/main.rs"),
            "use crate::util;\npub fn main() {}\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);
        assert_eq!(live_to(&index, &store, &util, "imports").unwrap().len(), 1);
        assert_eq!(
            index.relations_to(&util, "imports").len(),
            1,
            "no duplicate relation"
        );
        let before = store.count_objects().unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(store.count_objects().unwrap(), before);
    }

    #[test]
    fn deleted_files_lose_their_edges_including_pre_tombstone_deletions() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::write(
            src.path().join("src/gone.rs"),
            "use crate::keep;\npub fn g() {}\n",
        )
        .unwrap();
        fs::write(src.path().join("src/keep.rs"), "pub fn k() {}\n").unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        fs::remove_file(src.path().join("src/gone.rs")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        let r = refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(r.deleted, vec!["src/gone.rs".to_string()]);
        assert!(
            r.retracted >= 2,
            "contains + imports retracted: {}",
            r.retracted
        );
        let gone = StableId::derive(&["file", "src/gone.rs"]);
        let keep = StableId::derive(&["file", "src/keep.rs"]);
        let index = fresh_index(&store);
        assert!(live_from(&index, &store, &gone, "contains")
            .unwrap()
            .is_empty());
        assert!(live_to(&index, &store, &keep, "imports")
            .unwrap()
            .is_empty());

        // Healing: a live edge from an already-deleted file (as a store from
        // before tombstones would have) is retracted by the next refresh.
        let ghost = StableId::derive(&["symbol", "src/gone.rs", "fn", "ghost"]);
        store
            .put(&Object::Relation {
                from: gone.clone(),
                predicate: "contains".to_string(),
                to: ghost.clone(),
                source: "twin".to_string(),
                observed_at_ms: 1,
            })
            .unwrap();
        let r = refresh(&store, src.path(), "twin/app").unwrap();
        assert!(r.deleted.is_empty(), "already recorded as deleted");
        assert_eq!(r.retracted, 1, "the ghost edge is healed away");
        let index = fresh_index(&store);
        assert!(live_from(&index, &store, &gone, "contains")
            .unwrap()
            .is_empty());
        let before = store.count_objects().unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(store.count_objects().unwrap(), before);
    }

    #[test]
    fn taught_extensions_ingest_only_where_their_globs_reach() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("runs")).unwrap();
        fs::create_dir_all(src.path().join("stray")).unwrap();
        fs::write(src.path().join("runs/ledger.jsonl"), "{\"task\":\"t01\"}\n").unwrap();
        fs::write(src.path().join("stray/dump.jsonl"), "{}\n").unwrap();
        fs::write(src.path().join("notes.cfg"), "k=v\n").unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();

        // Teach a run-log kind whose extension only reaches runs/**.
        let tmpl = crate::templates::template_sid("run-log");
        store
            .put(&Object::Entity {
                id: tmpl.clone(),
                entity_kind: "template".to_string(),
                labels: BTreeMap::new(),
            })
            .unwrap();
        let now = now_ms();
        observe_src(&store, &tmpl, "applies_to", "run_log", "agent", now).unwrap();
        observe_src(&store, &tmpl, "capture", "runs/*.jsonl", "agent", now).unwrap();
        observe_src(&store, &tmpl, "extensions", "jsonl", "agent", now).unwrap();

        refresh(&store, src.path(), "twin/app").unwrap();
        let ns = store.namespace().unwrap();
        assert!(
            ns.contains_key("twin/app/runs/ledger.jsonl"),
            "in-glob jsonl ingested"
        );
        assert!(
            !ns.contains_key("twin/app/stray/dump.jsonl"),
            "stray jsonl invisible"
        );
        assert!(
            !ns.contains_key("twin/app/notes.cfg"),
            "untaught extension invisible"
        );
        // And it is captured as an artifact of the taught kind.
        let index = fresh_index(&store);
        let entity = StableId::derive(&["run_log", "twin/app", "ledger"]);
        assert!(latest(&index, &store, &entity, "content")
            .unwrap()
            .is_some());

        // Repo-level extensions apply everywhere (explicit opt-in).
        add_ingest_extensions(&store, "twin/app", &["cfg".to_string()]).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let ns = store.namespace().unwrap();
        assert!(ns.contains_key("twin/app/notes.cfg"));
    }

    #[test]
    fn compiled_kind_registry_captures_narrative_docs_and_stamps_contracts() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("docs/adr")).unwrap();
        fs::create_dir_all(src.path().join("docs/runbooks")).unwrap();
        fs::write(
            src.path().join("README.md"),
            "# The Project\n\nStart at src/main.rs.\n",
        )
        .unwrap();
        fs::write(
            src.path().join("docs/architecture.md"),
            "# Architecture\n\nLayers.\n",
        )
        .unwrap();
        fs::write(
            src.path().join("docs/adr/adr-001-x.md"),
            "# X\n\nStatus: accepted\n",
        )
        .unwrap();
        fs::write(
            src.path().join("docs/runbooks/release.md"),
            "# Cutting a release\n\nService: brain\n",
        )
        .unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::write(src.path().join("src/main.rs"), "pub fn main() {}\n").unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        // NO seed: compiled defaults alone must already capture.
        let r = refresh(&store, src.path(), "twin/app").unwrap();
        assert!(
            r.docs.len() >= 4,
            "README, architecture, adr, runbook: {:?}",
            r.docs
        );

        let index = fresh_index(&store);
        // README/docs/*.md become `doc` entities; the ADR path convention
        // keeps precedence (decision, not doc); runbook fields extract.
        let readme = StableId::derive(&["doc", "twin/app", "readme"]);
        assert_eq!(
            latest(&index, &store, &readme, "title").unwrap().as_deref(),
            Some("The Project")
        );
        assert_eq!(
            live_from(&index, &store, &readme, "mentions")
                .unwrap()
                .len(),
            1
        );
        let arch = StableId::derive(&["doc", "twin/app", "architecture"]);
        assert!(latest(&index, &store, &arch, "content").unwrap().is_some());
        let adr_as_doc = StableId::derive(&["doc", "twin/app", "adr-001-x"]);
        assert!(
            index.entity_nodes(&adr_as_doc).is_empty(),
            "builtin keeps the ADR"
        );
        let runbook = StableId::derive(&["runbook", "twin/app", "release"]);
        assert_eq!(
            latest(&index, &store, &runbook, "service")
                .unwrap()
                .as_deref(),
            Some("brain")
        );

        // README churn makes it stale at warn severity (narrative docs
        // describe the present).
        std::thread::sleep(std::time::Duration::from_millis(3));
        fs::write(
            src.path().join("src/main.rs"),
            "pub fn main() { /* v2 */ }\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert!(
            ins.stale_docs
                .iter()
                .any(|d| d.slug == "readme" && d.severity == Severity::Warn),
            "{:?}",
            ins.stale_docs
        );

        // After seeding, conformance runs and the judging contract version
        // is stamped on the artifact.
        crate::templates::seed(&store).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);
        let stamped = latest(&index, &store, &readme, "template_b3")
            .unwrap()
            .unwrap();
        let tmpl = crate::templates::template_sid("doc");
        assert_eq!(
            Some(stamped),
            latest(&index, &store, &tmpl, "contract_b3").unwrap(),
            "artifact records the contract that judged it"
        );

        // Idempotence across the whole pipeline.
        let before = store.count_objects().unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(store.count_objects().unwrap(), before);
    }

    #[test]
    fn same_run_moves_leave_a_renamed_to_trail() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::write(
            src.path().join("src/old.rs"),
            "pub fn stable_content() {}\n",
        )
        .unwrap();
        fs::write(src.path().join("src/twin_a.rs"), "pub fn dup() {}\n").unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        // Move: same bytes vanish here, appear there, in one refresh.
        fs::rename(src.path().join("src/old.rs"), src.path().join("src/new.rs")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);
        let old = StableId::derive(&["file", "src/old.rs"]);
        let new = StableId::derive(&["file", "src/new.rs"]);
        assert_eq!(
            latest(&index, &store, &old, "present").unwrap().as_deref(),
            Some("false")
        );
        let trail = live_from(&index, &store, &old, "renamed_to").unwrap();
        assert_eq!(trail.len(), 1);
        assert_eq!(trail[0].1, new);

        // Ambiguous matches (two identical new files) leave no trail.
        fs::write(src.path().join("src/twin_b.rs"), "pub fn dup() {}\n").unwrap();
        fs::rename(
            src.path().join("src/twin_a.rs"),
            src.path().join("src/twin_c.rs"),
        )
        .unwrap();
        // twin_a's bytes now exist at BOTH twin_b and twin_c (new paths).
        fs::write(src.path().join("src/twin_c.rs"), "pub fn dup() {}\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);
        let a = StableId::derive(&["file", "src/twin_a.rs"]);
        assert!(
            live_from(&index, &store, &a, "renamed_to")
                .unwrap()
                .is_empty(),
            "two candidates: no unique match, no trail"
        );

        // Idempotence still holds.
        let before = store.count_objects().unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(store.count_objects().unwrap(), before);
    }

    #[test]
    fn dropped_mentions_retract_but_mentions_of_deleted_files_stay() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::create_dir_all(src.path().join("docs/adr")).unwrap();
        fs::write(src.path().join("src/a.rs"), "pub fn a() {}\n").unwrap();
        fs::write(src.path().join("src/b.rs"), "pub fn b() {}\n").unwrap();
        fs::write(
            src.path().join("docs/adr/adr-001-x.md"),
            "# X\n\nStatus: accepted\n\nAbout src/a.rs and src/b.rs.\n",
        )
        .unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let doc = StableId::derive(&["decision", "twin/app", "adr-001-x"]);
        let a = StableId::derive(&["file", "src/a.rs"]);
        {
            let index = fresh_index(&store);
            assert_eq!(
                live_from(&index, &store, &doc, "mentions").unwrap().len(),
                2
            );
        }

        // The doc drops b.rs from its text: that mention is retracted, and
        // later churn in b.rs no longer makes the doc stale.
        std::thread::sleep(std::time::Duration::from_millis(3));
        fs::write(
            src.path().join("docs/adr/adr-001-x.md"),
            "# X\n\nStatus: accepted\n\nAbout src/a.rs only now.\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        {
            let index = fresh_index(&store);
            let live: Vec<StableId> = live_from(&index, &store, &doc, "mentions")
                .unwrap()
                .into_iter()
                .map(|(_, to)| to)
                .collect();
            assert_eq!(live, vec![a.clone()]);
        }
        std::thread::sleep(std::time::Duration::from_millis(3));
        fs::write(src.path().join("src/b.rs"), "pub fn b() { /* churn */ }\n").unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        {
            let index = fresh_index(&store);
            let ins = insights_with(&store, &index, "twin/app").unwrap();
            assert!(
                ins.stale_docs.is_empty(),
                "b.rs churn is not the doc's problem: {:?}",
                ins.stale_docs
            );
        }

        // Deleting a.rs while the text still names it keeps the mention
        // live — that mismatch belongs to coherence, not retraction.
        fs::remove_file(src.path().join("src/a.rs")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        refresh(&store, src.path(), "twin/app").unwrap();
        // Touch the doc so it is re-recorded (the sweep re-runs).
        std::thread::sleep(std::time::Duration::from_millis(3));
        fs::write(
            src.path().join("docs/adr/adr-001-x.md"),
            "# X\n\nStatus: accepted\n\nAbout src/a.rs only now. Still.\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);
        let live: Vec<StableId> = live_from(&index, &store, &doc, "mentions")
            .unwrap()
            .into_iter()
            .map(|(_, to)| to)
            .collect();
        assert_eq!(
            live,
            vec![a],
            "mention of the deleted-but-still-named file stays"
        );
    }
}

#[cfg(test)]
mod note_order_tests {
    use super::*;
    use brain_index::replay;

    fn fresh_index(store: &Store) -> MemIndex {
        let mut index = MemIndex::new();
        replay(store, &mut index).unwrap();
        index
    }

    /// Notes come back in the order they were written, and the log — not
    /// the clock — is what says so. Two notes written in the same
    /// millisecond are indistinguishable by timestamp, so sorting by time
    /// would put them in an arbitrary order; the put feed knows which came
    /// first. `notes()` looks that position up rather than walking the
    /// whole log, and this is the invariant that makes the shortcut legal.
    #[test]
    fn notes_keep_their_true_order_even_within_one_millisecond() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let subject = StableId::derive(&["file", "src/lib.rs"]);
        let other = StableId::derive(&["file", "src/other.rs"]);

        // One frozen timestamp for every note: the clock cannot break ties.
        let at = 1_700_000_000_000u64;
        for text in ["first", "second", "third", "fourth"] {
            store
                .put(&Object::Observation {
                    subject: subject.clone(),
                    property: "note".to_string(),
                    value: text.to_string(),
                    source: "agent".to_string(),
                    observed_at_ms: at,
                })
                .unwrap();
            // Interleave another subject's writes, so position in the feed
            // is not the same as position among this subject's own notes.
            store
                .put(&Object::Observation {
                    subject: other.clone(),
                    property: "note".to_string(),
                    value: format!("{text}-elsewhere"),
                    source: "agent".to_string(),
                    observed_at_ms: at,
                })
                .unwrap();
        }
        // A non-note observation on the same subject must not appear.
        store
            .put(&Object::Observation {
                subject: subject.clone(),
                property: "present".to_string(),
                value: "true".to_string(),
                source: "twin".to_string(),
                observed_at_ms: at,
            })
            .unwrap();

        let index = fresh_index(&store);
        let got: Vec<String> = notes(&index, &store, &subject)
            .unwrap()
            .into_iter()
            .map(|(_, text)| text)
            .collect();
        assert_eq!(got, ["first", "second", "third", "fourth"]);

        let elsewhere: Vec<String> = notes(&index, &store, &other)
            .unwrap()
            .into_iter()
            .map(|(_, text)| text)
            .collect();
        assert_eq!(
            elsewhere,
            [
                "first-elsewhere",
                "second-elsewhere",
                "third-elsewhere",
                "fourth-elsewhere"
            ]
        );
    }

    /// Reading the graph must stay cheap, and cheap here means two things
    /// that are easy to lose by accident: a pass touches each object's
    /// bytes about once, and a second pass touches none at all.
    ///
    /// Before the caches, one `insights` made 19,825 reads over 4,517
    /// distinct objects and re-read the event log 195 times. Those numbers
    /// were folklore until something asserted them.
    #[test]
    fn reading_the_graph_touches_each_object_about_once() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn a() {}\n").unwrap();
        std::fs::write(dir.path().join("src/other.rs"), "pub fn b() {}\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "# P\n\nSee src/lib.rs.\n").unwrap();
        let store = Store::open(dir.path().join(".brain")).unwrap();
        crate::templates::seed(&store).unwrap();
        refresh(&store, dir.path(), "twin/app").unwrap();

        // A fresh store: nothing is cached, so this is the honest cost.
        let store = Store::open(dir.path().join(".brain")).unwrap();
        let index = fresh_index(&store);
        let distinct = store.reads().from_disk;
        let before = store.reads();
        insights_with(&store, &index, "twin/app").unwrap();
        let first = store.reads();

        let served = first.served - before.served;
        let read = first.from_disk - before.from_disk;
        assert!(served > 0, "insights reads something");
        assert!(
            read <= distinct,
            "a pass should not need more object bytes than the graph has              ({read} byte-reads, {distinct} objects in the graph)"
        );

        // And again: everything it needs is already in hand.
        insights_with(&store, &index, "twin/app").unwrap();
        let second = store.reads();
        assert_eq!(
            second.from_disk, first.from_disk,
            "a second pass must not go to bytes at all"
        );
        assert!(
            second.served > first.served,
            "it did do the work again — it just did not pay for it twice"
        );
    }

    /// The put feed is memoised behind the log's byte length. Appending
    /// must be visible immediately — a stale feed would hide new objects
    /// from replay, which is how the whole index is built.
    #[test]
    fn the_memoised_put_feed_sees_what_was_just_written() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let subject = StableId::derive(&["file", "a.rs"]);

        assert!(store.put_history().unwrap().is_empty());
        let first = store
            .put(&Object::Observation {
                subject: subject.clone(),
                property: "note".to_string(),
                value: "one".to_string(),
                source: "agent".to_string(),
                observed_at_ms: 1,
            })
            .unwrap();
        assert_eq!(store.put_history().unwrap(), vec![first]);
        assert_eq!(store.put_position().unwrap().get(&first), Some(&0));

        let second = store
            .put(&Object::Observation {
                subject,
                property: "note".to_string(),
                value: "two".to_string(),
                source: "agent".to_string(),
                observed_at_ms: 2,
            })
            .unwrap();
        assert_eq!(store.put_history().unwrap(), vec![first, second]);
        assert_eq!(store.put_position().unwrap().get(&second), Some(&1));
    }
}
