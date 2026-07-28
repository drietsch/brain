//! Behavioural spec for the read-only projection.

use crate::body;
use crate::http;
use crate::state::{AppState, Config};
use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_observe::twin;
use brain_store::Store;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

struct Fixture {
    _workspace: tempfile::TempDir,
    _store_dir: tempfile::TempDir,
    state: AppState,
}

/// 32 recognisable bytes standing in for a screenshot.
const MEDIA_BYTES: &[u8; 32] = b"\x89PNG\r\n\x1a\n0123456789abcdefghijklmn";

/// A small but complete world: two crates where one uses the other, a
/// document that drifted, a decision, a feature, and a test.
fn fixture() -> Fixture {
    let workspace = tempfile::tempdir().unwrap();
    let store_dir = tempfile::tempdir().unwrap();
    let root = workspace.path();

    fs::create_dir_all(root.join("crates/core-lib/src")).unwrap();
    fs::create_dir_all(root.join("crates/app/src")).unwrap();
    fs::create_dir_all(root.join("docs/adr")).unwrap();
    fs::write(
        root.join("crates/core-lib/src/lib.rs"),
        "pub fn core_thing() {}\n",
    )
    .unwrap();
    fs::write(
        root.join("crates/app/src/lib.rs"),
        "use core_lib::core_thing;\npub fn app_thing() {}\n",
    )
    .unwrap();
    fs::write(
        root.join("README.md"),
        "# The project\n\nEverything starts in crates/core-lib/src/lib.rs.\n",
    )
    .unwrap();
    fs::write(
        root.join("docs/adr/adr-001-shape.md"),
        "# Keep the core small\n\nStatus: accepted\n\nAbout crates/core-lib/src/lib.rs.\n",
    )
    .unwrap();

    let store = Store::open(store_dir.path()).unwrap();
    brain_observe::templates::seed(&store).unwrap();
    twin::refresh(&store, root, "twin/app").unwrap();

    // A feature with two of four requirements met.
    brain_observe::features::add(&store, "twin/app", "core", "Core", "building").unwrap();
    let core_file = StableId::derive(&["file", "crates/core-lib/src/lib.rs"]);
    brain_observe::features::link(&store, "twin/app", "core", "implemented_by", &core_file)
        .unwrap();
    let decision = StableId::derive(&["decision", "twin/app", "adr-001-shape"]);
    brain_observe::features::link(&store, "twin/app", "core", "decided_by", &decision).unwrap();

    // A generated screenshot: media the browser must be able to stream.
    fs::create_dir_all(root.join("docs/generated/img")).unwrap();
    fs::write(root.join("docs/generated/img/shot.png"), MEDIA_BYTES).unwrap();

    // A browser test that failed and left evidence behind.
    fs::create_dir_all(root.join("e2e")).unwrap();
    fs::create_dir_all(root.join("test-results/login")).unwrap();
    fs::write(
        root.join("e2e/login.spec.ts"),
        "import { test } from '@playwright/test';\ntest('signs in', async () => {});\ntest('rejects a bad password', async () => {});\n",
    )
    .unwrap();
    fs::write(root.join("test-results/login/test-failed-1.png"), MEDIA_BYTES).unwrap();
    twin::refresh(&store, root, "twin/app").unwrap();

    // The code moves after the documents were written: drift.
    std::thread::sleep(std::time::Duration::from_millis(5));
    fs::write(
        root.join("crates/core-lib/src/lib.rs"),
        "pub fn core_thing() { /* changed */ }\n",
    )
    .unwrap();
    twin::refresh(&store, root, "twin/app").unwrap();

    // Declared as an asset, the way the docs pipeline declares its own.
    let repo = StableId::derive(&["repo", "twin/app"]);
    brain_observe::assets::add(
        &store,
        "twin/app",
        "docs/generated/img/shot.png",
        &repo,
        &[],
        Some("image"),
    )
    .unwrap();

    // A Playwright protocol: one pass, one failure with its screenshot.
    let report = r#"{"stats":{"duration":2400},"suites":[{"title":"e2e/login.spec.ts","file":"e2e/login.spec.ts","specs":[
      {"title":"signs in","file":"e2e/login.spec.ts","line":2,
       "tests":[{"projectName":"","results":[{"status":"passed","duration":410}]}]},
      {"title":"rejects a bad password","file":"e2e/login.spec.ts","line":3,
       "tests":[{"projectName":"","results":[{"status":"failed","duration":1200,
         "error":{"message":"Expected 401, got 500"},
         "attachments":[{"name":"screenshot","path":"test-results/login/test-failed-1.png","contentType":"image/png"}]}]}]}
    ]}]}"#;
    let parsed = brain_observe::testing::parse_report(report);
    brain_observe::testing::record_run_in(&store, root, "twin/app", &parsed, report).unwrap();

    let state = AppState::new(Config {
        store_root: store_dir.path().to_path_buf(),
        content_root: root.to_path_buf(),
        prefix: "twin/app".to_string(),
        bind: "127.0.0.1".to_string(),
        port: 0,
    })
    .unwrap();

    Fixture {
        _workspace: workspace,
        _store_dir: store_dir,
        state,
    }
}

#[test]
fn queries_never_write_and_every_view_names_its_snapshot() {
    let f = fixture();
    let store = Store::open(f.state.config.store_root.clone()).unwrap();
    let before = store.count_objects().unwrap();

    let now = f.state.read(crate::query::now::build).unwrap();
    let library = f
        .state
        .read(|loaded| crate::query::library::build(loaded, "decisions", ""))
        .unwrap();
    let map = f
        .state
        .read(|loaded| crate::query::map::build(loaded, "tests"))
        .unwrap();
    let timeline = f
        .state
        .read(|loaded| crate::query::timeline::build(loaded, 20))
        .unwrap();

    assert_eq!(store.count_objects().unwrap(), before, "Eyes wrote nothing");
    for snapshot in [
        &now.snapshot,
        &library.snapshot,
        &map.snapshot,
        &timeline.snapshot,
    ] {
        assert_eq!(snapshot.prefix, "twin/app");
        assert!(snapshot.cursor > 0);
        assert!(snapshot.head.is_some());
    }
}

#[test]
fn now_speaks_in_sentences_and_names_the_fix() {
    let f = fixture();
    let now = f.state.read(crate::query::now::build).unwrap();

    assert!(now.headline.ends_with('.'), "headline: {}", now.headline);
    // README mentions the file that changed underneath it.
    let drift = now
        .needs_you
        .iter()
        .find(|concern| concern.title.contains("may be wrong"))
        .expect("the drifted document is surfaced");
    assert!(
        drift.reason.contains("crates/core-lib/src/lib.rs"),
        "the reason names the file: {}",
        drift.reason
    );
    assert!(
        drift
            .fix_command
            .as_deref()
            .is_some_and(|command| command.starts_with("brain artifact ack twin/app")),
        "a read-only view still tells you the command: {:?}",
        drift.fix_command
    );

    // No machine vocabulary in the primary flow.
    let prose = format!(
        "{} {} {}",
        now.headline,
        now.subhead,
        now.needs_you
            .iter()
            .map(|c| format!("{} {}", c.title, c.reason))
            .collect::<Vec<_>>()
            .join(" ")
    );
    for jargon in ["b3:", "cursor", "StableId", "predicate", "entity_kind"] {
        assert!(!prose.contains(jargon), "leaked {jargon:?} into: {prose}");
    }
}

#[test]
fn attention_cards_drop_reasons_a_person_cannot_use() {
    let f = fixture();
    let now = f.state.read(crate::query::now::build).unwrap();
    for card in &now.attention {
        assert!(!card.reasons.is_empty(), "a card with nothing to say is noise");
        for reason in &card.reasons {
            assert!(!reason.starts_with("hub "), "raw reason: {reason}");
            assert!(!reason.starts_with("churn "), "raw reason: {reason}");
        }
    }
}

#[test]
fn shelves_present_content_by_kind() {
    let f = fixture();
    let view = f
        .state
        .read(|loaded| crate::query::library::build(loaded, "decisions", ""))
        .unwrap();
    let decision = view
        .items
        .iter()
        .find(|item| item.title == "Keep the core small")
        .expect("the decision is on the shelf, titled by what it says");
    assert!(decision.label.contains("adr-001-shape"));
    assert!(decision.facts.iter().any(|fact| fact.contains("accepted")));
    assert!(decision.excerpt.is_some(), "a reading list shows what it says");

    let features = f
        .state
        .read(|loaded| crate::query::library::build(loaded, "features", ""))
        .unwrap();
    let feature = features.items.first().expect("the feature is on its shelf");
    let coverage = feature.coverage.as_ref().expect("features carry a coverage strip");
    assert_eq!(coverage.len(), 4, "four definition-of-done cells");
    assert_eq!(coverage.iter().filter(|cell| cell.met).count(), 2);
    assert!(feature
        .state
        .as_deref()
        .is_some_and(|state| state.contains("2 of 4")));
}

#[test]
fn concepts_explain_the_vocabulary_including_what_the_brain_learned() {
    let f = fixture();
    let view = f.state.read(crate::query::library::concepts).unwrap();
    let decision = view
        .concepts
        .iter()
        .find(|concept| concept.kind == "decision")
        .expect("decisions are a concept");
    assert_eq!(decision.purpose, "Architecture decision record");
    assert!(decision.requires.contains(&"status".to_string()));
    assert!(decision.placement_note.contains("file"));
    assert!(decision.enforcement_note.contains("recorded"));
    assert!(decision.count >= 1);
}

#[test]
fn every_test_is_listed_with_its_verdict_and_its_evidence() {
    let f = fixture();
    let view = f.state.read(crate::query::tests::build).unwrap();

    assert!(
        view.headline.contains("1 of 2 tests failed"),
        "the headline leads with the failure: {}",
        view.headline
    );

    // Every case, not a summary — and the failing one is first.
    assert_eq!(view.cases.len(), 2);
    let failing = &view.cases[0];
    assert_eq!(failing.result, "failing");
    assert!(failing.name.contains("rejects a bad password"));
    assert_eq!(
        failing.error.as_deref(),
        Some("Expected 401, got 500"),
        "the reason it failed, not just that it failed"
    );
    assert_eq!(failing.duration.as_deref(), Some("1.2 seconds"));
    assert_eq!(failing.framework.as_deref(), Some("Playwright"));
    assert_eq!(
        failing.file.as_ref().map(|f| f.label.as_str()),
        Some("e2e/login.spec.ts"),
        "the case knows where it lives"
    );

    // The screenshot the failure produced is attached to the case.
    let shot = failing
        .attachments
        .first()
        .expect("a failing browser test shows its screenshot");
    assert_eq!(shot.noun, "screenshot");
    assert_eq!(shot.path, "test-results/login/test-failed-1.png");

    // The run itself is a thing you can open.
    let protocol = view.protocols.first().expect("the run is listed");
    assert_eq!((protocol.total, protocol.passed, protocol.failed), (2, 1, 1));
    assert_eq!(protocol.verdict, "1 of 2 failed");
    assert_eq!(protocol.source, "from Playwright");
    assert_eq!(protocol.duration.as_deref(), Some("2.4 seconds"));
    assert!(
        protocol.named.iter().any(|c| c.result == "failing"),
        "a run names what failed"
    );

    // The suite the twin classified, with its framework.
    let suite = view
        .suites
        .iter()
        .find(|s| s.path == "e2e/login.spec.ts")
        .expect("the spec file is a suite");
    assert_eq!(suite.framework_label, "Playwright");
    assert_eq!(suite.declared, 2);
    assert!(view
        .frameworks
        .iter()
        .any(|f| f.label == "Playwright" && f.files == 1));
}

#[test]
fn work_names_who_did_something_and_stays_quiet_when_nobody_did() {
    let f = fixture();
    let view = f.state.read(crate::query::work::build).unwrap();
    assert!(view.sessions.is_empty());
    assert!(
        view.sessions_hint.as_deref().is_some_and(|hint| hint.contains("Import them")),
        "an empty surface says how to fill it: {:?}",
        view.sessions_hint
    );
    assert_eq!(
        view.sessions_hint_command.as_deref(),
        Some("brain sessions import . --prefix twin/app"),
        "and the command lives in a command field, not in the sentence"
    );
    assert!(view.headline.contains("Nothing is in flight"));
}

#[test]
fn evidence_shows_when_a_claim_outruns_its_proof() {
    let f = fixture();
    let view = f.state.read(crate::query::evidence::build).unwrap();

    // The fixture's feature links a decision and an implementation but is
    // not done; the claim must not read as supported.
    let feature = view
        .claims
        .iter()
        .find(|claim| claim.category == "features")
        .expect("the feature makes a claim");
    assert!(!feature.supported);
    assert!(
        feature.verdict.contains("2 of 4"),
        "the verdict counts what is missing: {}",
        feature.verdict
    );
    assert!(
        feature.proof.iter().any(|p| p.text.contains("nothing is linked as")),
        "the proof names the gap"
    );

    // The failing run is evidence, and it says what kind of evidence.
    let run = view
        .claims
        .iter()
        .find(|claim| claim.category == "tests")
        .expect("the run is evidence");
    assert!(!run.supported, "the suite failed");
    assert!(run.proof.iter().any(|p| p
        .basis
        .as_deref()
        .is_some_and(|b| b.contains("run and observed"))));

    assert!(view.headline.contains("cannot show proof"), "{}", view.headline);
}

#[test]
fn media_carries_its_provenance_and_its_freshness() {
    let f = fixture();
    let root = f.state.config.content_root.clone();
    let view = f
        .state
        .read(|loaded| crate::query::media::build(loaded, Some(&root)))
        .unwrap();

    let shot = view
        .items
        .iter()
        .find(|item| item.label == "shot.png")
        .expect("the declared screenshot is media");
    assert_eq!(shot.noun, "screenshot");
    // No tour exists in the fixture, so nothing is invented for one.
    assert!(view.tour.is_none());
}

#[test]
fn the_map_rolls_files_up_into_modules_and_orders_them_by_dependency() {
    let f = fixture();
    let view = f
        .state
        .read(|loaded| crate::query::map::build(loaded, "tests"))
        .unwrap();
    let app = view
        .blocks
        .iter()
        .find(|block| block.path == "crates/app")
        .expect("the app crate is a block");
    let core = view
        .blocks
        .iter()
        .find(|block| block.path == "crates/core-lib")
        .expect("the core crate is a block");
    assert!(
        app.layer > core.layer,
        "what depends on something sits above it: app {} core {}",
        app.layer,
        core.layer
    );
    assert!(view
        .edges
        .iter()
        .any(|edge| edge.from == "crates/app" && edge.to == "crates/core-lib"));
    assert!(view.blocks.len() < 10, "a map a person can read");
    assert!(app.sentence.contains("test"), "the tests lens explains itself");
}

#[test]
fn a_thing_leads_with_its_body_and_places_its_neighbours() {
    let f = fixture();
    let sid = StableId::derive(&["file", "crates/core-lib/src/lib.rs"]);
    let root = f.state.config.content_root.clone();
    let view = f
        .state
        .read(|loaded| crate::query::thing::build(loaded, &sid.to_string(), Some(&root)))
        .unwrap();

    assert_eq!(view.noun, "file");
    let body = view.body.expect("the file's bytes are readable");
    assert!(body.text.unwrap().contains("core_thing"));
    assert!(body.verified, "the graph recorded a hash to check against");

    // crates/app uses core, so core is depended upon.
    assert!(
        view.neighborhood
            .downstream
            .iter()
            .any(|entry| entry.label.contains("app")),
        "what depends on this sits downstream: {:?}",
        view.neighborhood.downstream
    );
    assert!(view.neighborhood.sentence.contains("depend"));
    assert!(!view.versions.is_empty(), "the file has a version history");
    assert!(view.versions[0].current);
    assert!(
        view.history.iter().any(|entry| entry.text == "the file changed"),
        "history reads as events, not property names: {:?}",
        view.history
    );
    // Machine identity is available, but only under details.
    assert!(view.details.iter().any(|(label, _)| label == "Stable id"));
}

#[test]
fn a_decision_reads_as_a_document() {
    let f = fixture();
    let sid = StableId::derive(&["decision", "twin/app", "adr-001-shape"]);
    let root = f.state.config.content_root.clone();
    let view = f
        .state
        .read(|loaded| crate::query::thing::build(loaded, &sid.to_string(), Some(&root)))
        .unwrap();
    assert_eq!(view.title, "Keep the core small");
    assert_eq!(view.noun, "decision");
    let body = view.body.expect("decisions are read, not inspected");
    assert_eq!(body.format, "markdown");
    assert!(body.text.unwrap().contains("Keep the core small"));
    // It mentions the file that changed after it was written.
    assert!(view
        .facts
        .iter()
        .any(|fact| fact.text.contains("changed after this was written")));
}

#[test]
fn timeline_groups_activity_into_episodes() {
    let f = fixture();
    let view = f
        .state
        .read(|loaded| crate::query::timeline::build(loaded, 20))
        .unwrap();
    assert!(!view.episodes.is_empty());
    let first = &view.episodes[0];
    assert!(
        first
            .title
            .chars()
            .next()
            .is_some_and(|c| c.is_uppercase() || c.is_ascii_digit()),
        "episodes are titled sentences: {}",
        first.title
    );
    assert!(
        !first.title.contains("content_b3") && !first.title.contains("observation"),
        "no property names in a title: {}",
        first.title
    );
    assert!(!first.when.is_empty());
    // The second refresh changed exactly one file.
    assert!(view
        .episodes
        .iter()
        .any(|episode| episode.title.contains("changed")));
}

#[test]
fn find_matches_names_and_says_why() {
    let f = fixture();
    let view = f
        .state
        .read(|loaded| crate::query::find::build(loaded, "core", 20))
        .unwrap();
    assert_eq!(view.query, "core", "the query echoes what was typed");
    assert!(!view.hits.is_empty());
    assert!(view.hits.iter().all(|hit| !hit.because.is_empty()));
    assert!(view
        .hits
        .iter()
        .any(|hit| hit.target.label.contains("core")));
}

#[test]
fn prefixes_do_not_bleed_into_their_neighbours() {
    let f = fixture();
    let store = Store::open(f.state.config.store_root.clone()).unwrap();
    // A namespace that merely starts with the same characters.
    let sid = StableId::derive(&["file", "elsewhere.rs"]);
    let node = store
        .put(&Object::Entity {
            id: sid.clone(),
            entity_kind: "source_file".to_string(),
            labels: BTreeMap::from([("path".to_string(), "elsewhere.rs".to_string())]),
        })
        .unwrap();
    store.bind("twin/apphosted/elsewhere.rs", node).unwrap();

    let view = f
        .state
        .read(|loaded| crate::query::map::build(loaded, "tests"))
        .unwrap();
    assert!(
        !view
            .blocks
            .iter()
            .any(|block| block.path.contains("elsewhere")),
        "twin/app must not swallow twin/apphosted"
    );
    assert!(crate::query::in_prefix("twin/app/x", "twin/app"));
    assert!(crate::query::in_prefix("twin/app", "twin/app"));
    assert!(!crate::query::in_prefix("twin/apphosted/x", "twin/app"));
}

#[test]
fn workspace_bytes_are_contained_and_verification_is_never_claimed_falsely() {
    let workspace = tempfile::tempdir().unwrap();
    let store_dir = tempfile::tempdir().unwrap();
    let root = workspace.path();
    fs::create_dir_all(root.join("docs/assets")).unwrap();
    fs::write(root.join("docs/assets/shot.png"), b"\x89PNG\r\n\x1a\nbytes").unwrap();

    let store = Store::open(store_dir.path()).unwrap();
    twin::refresh(&store, root, "twin/app").unwrap();
    // Written after the twin looked: the graph has no hash for it.
    fs::write(root.join("untwinned.txt"), b"loose bytes").unwrap();

    // An entity whose path was never twinned: nothing to verify against.
    let sid = StableId::derive(&["asset", "twin/app", "loose"]);
    store
        .put(&Object::Entity {
            id: sid.clone(),
            entity_kind: "asset".to_string(),
            labels: BTreeMap::from([
                ("prefix".to_string(), "twin/app".to_string()),
                ("slug".to_string(), "loose".to_string()),
                ("path".to_string(), "untwinned.txt".to_string()),
            ]),
        })
        .unwrap();

    let state = AppState::new(Config {
        store_root: store_dir.path().to_path_buf(),
        content_root: root.to_path_buf(),
        prefix: "twin/app".to_string(),
        ..Config::default()
    })
    .unwrap();

    // The hash-less body is served, but never described as verified.
    let view = state
        .read(|loaded| {
            let labels = crate::query::labels_of(&loaded.index, &loaded.store, &sid);
            body::resolve(loaded, &sid, "asset", &labels, Some(root)).map(|r| r.view)
        })
        .unwrap();
    assert!(!view.verified, "no recorded hash means no verification claim");
    assert!(
        view.origin.contains("no record to check it against"),
        "and it says so: {}",
        view.origin
    );

    // A twinned file does get verified.
    let shot = StableId::derive(&["file", "docs/assets/shot.png"]);
    let view = state
        .read(|loaded| {
            let labels = crate::query::labels_of(&loaded.index, &loaded.store, &shot);
            body::resolve(loaded, &shot, "source_file", &labels, Some(root)).map(|r| r.view)
        })
        .unwrap();
    assert!(view.verified);

    // Changed bytes are refused rather than shown.
    fs::write(root.join("docs/assets/shot.png"), b"different").unwrap();
    let result = state.read(|loaded| {
        let labels = crate::query::labels_of(&loaded.index, &loaded.store, &shot);
        body::resolve(loaded, &shot, "source_file", &labels, Some(root)).map(|r| r.view)
    });
    assert!(result.is_err());

    // Path containment.
    assert!(body::safe_content_path(root, "../untwinned.txt").is_err());
    assert!(body::safe_content_path(root, "/etc/passwd").is_err());
    assert!(body::safe_content_path(root, "docs/assets/shot.png").is_ok());
}

#[test]
fn url_parsing_handles_what_a_browser_actually_sends() {
    assert_eq!(http::split_url("/api/thing?id=x"), ("/api/thing", "id=x"));
    assert_eq!(http::split_url("/api/now"), ("/api/now", ""));
    assert_eq!(http::query_param("id=abc&q=hi", "q"), Some("hi"));
    assert_eq!(http::query_param("id=abc", "missing"), None);
    assert_eq!(http::percent_decode("a%20b"), "a b");
    assert_eq!(http::percent_decode("a+b"), "a b");
    // The traversal defence runs after decoding, so this is still refused.
    assert_eq!(http::percent_decode("%2e%2e%2fetc"), "../etc");
    assert!(body::safe_content_path(Path::new("."), &http::percent_decode("%2e%2e%2fetc")).is_err());
}

#[test]
fn the_view_refreshes_only_when_the_graph_moves() {
    let f = fixture();
    let first = f.state.snapshot().unwrap();
    let again = f.state.snapshot().unwrap();
    assert_eq!(first.cursor, again.cursor);

    // Writing to the store advances the cursor; the next read sees it.
    let store = Store::open(f.state.config.store_root.clone()).unwrap();
    let repo = StableId::derive(&["repo", "twin/app"]);
    twin::add_note(&store, &repo, "a note from a session").unwrap();
    let after = f.state.snapshot().unwrap();
    assert!(after.cursor > first.cursor, "the held view caught up");
}

/// Everything a person reads in the primary flow, gathered so one test can
/// check the whole vocabulary at once.
fn prose_of(f: &Fixture) -> String {
    let mut prose = Vec::new();
    let now = f.state.read(crate::query::now::build).unwrap();
    prose.push(now.headline.clone());
    prose.push(now.subhead.clone());
    for concern in &now.needs_you {
        prose.push(concern.title.clone());
        prose.push(concern.reason.clone());
    }
    prose.push(now.since.summary.clone());
    for episode in &now.since.episodes {
        prose.push(episode.title.clone());
        prose.extend(episode.facts.clone());
    }
    for card in &now.attention {
        prose.extend(card.reasons.clone());
    }
    prose.push(now.proof.sentence.clone());
    for group in &now.proof.groups {
        prose.push(group.label.clone());
        prose.extend(group.cells.iter().map(|cell| cell.text.clone()));
    }
    for shelf in ["decisions", "plans", "documents", "features"] {
        let view = f
            .state
            .read(|loaded| crate::query::library::build(loaded, shelf, ""))
            .unwrap();
        prose.push(view.note.clone());
        for item in &view.items {
            prose.push(item.title.clone());
            prose.extend(item.state.clone());
            prose.extend(item.state_note.clone());
        }
    }
    let map = f
        .state
        .read(|loaded| crate::query::map::build(loaded, "tests"))
        .unwrap();
    prose.push(map.sentence.clone());
    prose.push(map.lens_note.clone());
    for block in &map.blocks {
        prose.push(block.sentence.clone());
        prose.extend(block.facts.clone());
    }
    let timeline = f
        .state
        .read(|loaded| crate::query::timeline::build(loaded, 30))
        .unwrap();
    for episode in &timeline.episodes {
        prose.push(episode.title.clone());
        prose.extend(episode.facts.clone());
    }
    let concepts = f.state.read(crate::query::library::concepts).unwrap();
    for concept in &concepts.concepts {
        prose.push(concept.placement_note.clone());
        prose.push(concept.enforcement_note.clone());
        prose.push(concept.rot_note.clone());
    }

    // The surfaces added in v3 answer to the same rule.
    let tests = f.state.read(crate::query::tests::build).unwrap();
    prose.push(tests.headline.clone());
    for case in &tests.cases {
        prose.push(case.result.clone());
        prose.extend(case.note.clone());
        prose.extend(case.error.clone());
        prose.extend(case.duration.clone());
        for attachment in &case.attachments {
            prose.push(attachment.noun.clone());
        }
    }
    for run in &tests.protocols {
        prose.push(run.verdict.clone());
        prose.push(run.source.clone());
        prose.extend(run.evidence.clone());
    }
    for suite in &tests.suites {
        prose.push(suite.note.clone());
        prose.push(suite.framework_label.clone());
    }

    let work = f.state.read(crate::query::work::build).unwrap();
    prose.push(work.headline.clone());
    prose.extend(work.sessions_hint.clone());
    for session in &work.sessions {
        prose.push(session.state.clone());
        prose.push(session.ran_for.clone());
        prose.push(session.agent_label.clone());
        for tool in &session.tools {
            prose.push(tool.label.clone());
        }
    }
    for item in work.changes.iter().chain(work.plans.iter()) {
        prose.push(item.stage.clone());
        prose.push(item.note.clone());
    }

    let evidence = f.state.read(crate::query::evidence::build).unwrap();
    prose.push(evidence.headline.clone());
    for category in &evidence.categories {
        prose.push(category.label.clone());
        prose.push(category.note.clone());
    }
    for claim in &evidence.claims {
        prose.push(claim.claim.clone());
        prose.push(claim.verdict.clone());
        for proof in &claim.proof {
            prose.push(proof.text.clone());
            prose.extend(proof.basis.clone());
        }
    }

    let root = f.state.config.content_root.clone();
    let media = f
        .state
        .read(|loaded| crate::query::media::build(loaded, Some(&root)))
        .unwrap();
    prose.push(media.headline.clone());
    for item in &media.items {
        prose.push(item.state.clone());
        prose.push(item.state_note.clone());
        prose.push(item.noun.clone());
    }

    let mri = f.state.read(|loaded| loaded.mri()).unwrap();
    prose.push(mri.headline.clone());
    for cluster in &mri.clusters {
        prose.push(cluster.label.clone());
        prose.push(cluster.note.clone());
    }
    prose.join("\n")
}

#[test]
fn nothing_a_person_reads_speaks_machine() {
    let f = fixture();
    let prose = prose_of(&f);
    // Vocabulary that only means something if you have read the ADRs.
    for jargon in [
        "b3:", "sid:", "cursor", "StableId", "predicate", "entity_kind", "observation",
        "content_b3", "stale_docs", "put_history", "hub ", "churn ", "MemIndex", "prefix ",
        "applies_to", "conforms_to", "recorded_in", "live_from", "untested hub",
    ] {
        assert!(
            !prose.contains(jargon),
            "{jargon:?} reached a human surface:\n{prose}"
        );
    }
    // And the sentences are sentences.
    assert!(prose.contains("may be wrong") || prose.contains("current"));
}

#[test]
fn a_warm_view_answers_without_rescanning_the_store() {
    // A store big enough that a full pass per request would be obvious:
    // the old implementation made roughly nine of them per dossier.
    let workspace = tempfile::tempdir().unwrap();
    let store_dir = tempfile::tempdir().unwrap();
    let root = workspace.path();
    fs::create_dir_all(root.join("crates/big/src")).unwrap();
    for index in 0..120 {
        fs::write(
            root.join(format!("crates/big/src/mod{index}.rs")),
            format!("pub fn thing_{index}() {{}}\npub struct Thing{index};\n"),
        )
        .unwrap();
    }
    let store = Store::open(store_dir.path()).unwrap();
    brain_observe::templates::seed(&store).unwrap();
    twin::refresh(&store, root, "twin/app").unwrap();
    assert!(store.count_objects().unwrap() > 500, "a store worth timing");

    let state = AppState::new(Config {
        store_root: store_dir.path().to_path_buf(),
        content_root: root.to_path_buf(),
        prefix: "twin/app".to_string(),
        ..Config::default()
    })
    .unwrap();

    let sid = StableId::derive(&["file", "crates/big/src/mod7.rs"]);
    let started = std::time::Instant::now();
    for _ in 0..20 {
        let root = state.config.content_root.clone();
        state
            .read(|loaded| crate::query::thing::build(loaded, &sid.to_string(), Some(&root)))
            .unwrap();
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(4),
        "twenty warm reads took {elapsed:?} — something is scanning the store again"
    );
}

#[test]
fn the_server_answers_over_a_real_socket_and_refuses_the_rest() {
    use std::io::{Read, Write};
    let f = fixture();
    let (server, address, state) = crate::http::bind(Config {
        port: 0,
        ..f.state.config.clone()
    })
    .unwrap();
    std::thread::spawn(move || {
        let _ = crate::http::run(server, state);
    });

    let request = |line: &str| -> String {
        let mut socket = std::net::TcpStream::connect(&address).unwrap();
        socket
            .write_all(format!("{line}\r\nHost: localhost\r\nConnection: close\r\n\r\n").as_bytes())
            .unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        response
    };

    assert!(request("GET / HTTP/1.1").starts_with("HTTP/1.1 200"));
    let snapshot = request("GET /api/snapshot HTTP/1.1");
    assert!(snapshot.contains("\"prefix\":\"twin/app\""));
    assert!(snapshot.contains("X-Content-Type-Options: nosniff"));
    assert!(snapshot.contains("Content-Security-Policy"));

    // Unknown routes and write attempts get a sentence, not a stack trace.
    let missing = request("GET /api/nope HTTP/1.1");
    assert!(missing.starts_with("HTTP/1.1 404"), "{missing}");
    assert!(missing.contains("nothing at that address"));
    let posted = request("POST /api/now HTTP/1.1");
    assert!(posted.starts_with("HTTP/1.1 405"), "{posted}");
    assert!(posted.contains("never writes"));

    // A thing that does not exist is a 404, not a 500.
    let unknown = request("GET /api/thing?id=sid:doesnotexist HTTP/1.1");
    assert!(unknown.starts_with("HTTP/1.1 404"), "{unknown}");
}

#[test]
fn media_can_be_streamed_and_seeked() {
    use std::io::{Read, Write};
    let f = fixture();
    let asset = brain_observe::assets::asset_sid("twin/app", "shot");
    let (server, address, state) = crate::http::bind(Config {
        port: 0,
        ..f.state.config.clone()
    })
    .unwrap();
    std::thread::spawn(move || {
        let _ = crate::http::run(server, state);
    });

    // Raw bytes, so a Range slice can be compared exactly.
    let fetch = |range: Option<&str>| -> (String, Vec<u8>) {
        let mut socket = std::net::TcpStream::connect(&address).unwrap();
        let range = range
            .map(|r| format!("Range: {r}\r\n"))
            .unwrap_or_default();
        socket
            .write_all(
                format!(
                    "GET /api/body?id={} HTTP/1.1\r\nHost: localhost\r\n{range}Connection: close\r\n\r\n",
                    asset.0
                )
                .as_bytes(),
            )
            .unwrap();
        let mut raw = Vec::new();
        socket.read_to_end(&mut raw).unwrap();
        let split = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("headers end");
        (
            String::from_utf8_lossy(&raw[..split]).to_string(),
            raw[split + 4..].to_vec(),
        )
    };

    // Whole file: advertises that ranges are available at all, which is
    // what a browser checks before it will let anyone scrub a video.
    let (head, body) = fetch(None);
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert!(head.contains("Accept-Ranges: bytes"), "{head}");
    assert!(head.contains("Content-Type: image/png"), "{head}");
    assert_eq!(body, MEDIA_BYTES.to_vec());

    // A middle slice.
    let (head, body) = fetch(Some("bytes=8-11"));
    assert!(head.starts_with("HTTP/1.1 206"), "{head}");
    assert!(head.contains("Content-Range: bytes 8-11/32"), "{head}");
    assert_eq!(body, b"0123".to_vec());

    // Open-ended, and suffix — both forms browsers actually send.
    let (head, body) = fetch(Some("bytes=28-"));
    assert!(head.contains("Content-Range: bytes 28-31/32"), "{head}");
    assert_eq!(body, b"klmn".to_vec());
    let (head, body) = fetch(Some("bytes=-4"));
    assert!(head.contains("Content-Range: bytes 28-31/32"), "{head}");
    assert_eq!(body, b"klmn".to_vec());

    // Past the end is refused, not silently answered with the whole file.
    let (head, _) = fetch(Some("bytes=99-200"));
    assert!(head.starts_with("HTTP/1.1 416"), "{head}");
    assert!(head.contains("Content-Range: bytes */32"), "{head}");
}

#[test]
fn byte_ranges_are_read_the_way_http_defines_them() {
    use crate::http::parse_range;
    assert_eq!(parse_range("bytes=0-0", 32), Some(Ok((0, 0))));
    assert_eq!(parse_range("bytes=8-11", 32), Some(Ok((8, 11))));
    assert_eq!(parse_range("bytes=28-", 32), Some(Ok((28, 31))));
    assert_eq!(parse_range("bytes=-4", 32), Some(Ok((28, 31))));
    // An end past the file is clamped; a start past it is not satisfiable.
    assert_eq!(parse_range("bytes=30-999", 32), Some(Ok((30, 31))));
    assert_eq!(parse_range("bytes=32-40", 32), Some(Err(())));
    assert_eq!(parse_range("bytes=-0", 32), Some(Err(())));
    assert_eq!(parse_range("bytes=0-10", 0), Some(Err(())));
    // Nothing usable: send the whole body rather than guess.
    assert_eq!(parse_range("bytes=0-1,4-5", 32), None);
    assert_eq!(parse_range("items=0-1", 32), None);
    assert_eq!(parse_range("bytes=abc-def", 32), None);
}


#[test]
fn the_anatomy_is_stable_and_keeps_every_node() {
    let f = fixture();
    let first = f.state.read(|loaded| loaded.mri()).unwrap();
    let second = f.state.read(|loaded| loaded.mri()).unwrap();

    // A layout that moved between reads would make motion meaningless.
    assert_eq!(first.nodes.len(), second.nodes.len());
    for (a, b) in first.nodes.iter().zip(second.nodes.iter()) {
        assert_eq!((a.id.as_str(), a.x, a.y, a.z), (b.id.as_str(), b.x, b.y, b.z));
    }

    // Nothing is dropped: every entity the graph holds under this prefix
    // is in the payload, which is the promise the old whole-graph view
    // broke by silently discarding three quarters of it.
    let files = f
        .state
        .read(|loaded| crate::query::present_files(&loaded.index, &loaded.store, "twin/app"))
        .unwrap();
    for (path, sid) in &files {
        assert!(
            first.nodes.iter().any(|node| node.id == sid.to_string()),
            "{path} is missing from the anatomy"
        );
    }
    assert_eq!(first.levels.iter().sum::<usize>(), first.nodes.len());

    // Height means dependency depth: the crate that uses another sits
    // above it.
    let y_of = |path: &str| {
        let sid = StableId::derive(&["file", path]);
        first
            .nodes
            .iter()
            .find(|node| node.id == sid.to_string())
            .map(|node| node.y)
            .unwrap_or_default()
    };
    assert!(
        y_of("crates/app/src/lib.rs") > y_of("crates/core-lib/src/lib.rs"),
        "what depends on something sits above it"
    );

    // Edges point at real nodes.
    for edge in &first.edges {
        assert!((edge.a as usize) < first.nodes.len());
        assert!((edge.b as usize) < first.nodes.len());
    }
}

#[test]
fn every_surface_is_a_read() {
    let f = fixture();
    let store = Store::open(f.state.config.store_root.clone()).unwrap();
    let before = store.count_objects().unwrap();
    let root = f.state.config.content_root.clone();

    // A crawl of everything a person can open.
    f.state.read(crate::query::now::build).unwrap();
    f.state.read(crate::query::work::build).unwrap();
    f.state.read(crate::query::tests::build).unwrap();
    f.state.read(crate::query::evidence::build).unwrap();
    f.state
        .read(|loaded| crate::query::media::build(loaded, Some(&root)))
        .unwrap();
    f.state.read(crate::query::library::concepts).unwrap();
    f.state.read(|loaded| loaded.mri()).unwrap();
    for lens in ["attention", "tests", "recent"] {
        f.state
            .read(|loaded| crate::query::map::build(loaded, lens))
            .unwrap();
    }
    f.state
        .read(|loaded| crate::query::timeline::build(loaded, 50))
        .unwrap();

    assert_eq!(
        store.count_objects().unwrap(),
        before,
        "Eyes wrote something while only being looked at"
    );
}

#[test]
fn a_part_that_is_not_ready_sinks_its_parent() {
    let f = fixture();
    let store = Store::open(f.state.config.store_root.clone()).unwrap();

    // The fixture's `core` feature is 2 of 4. Give it a part that is
    // fully linked, and one that has nothing.
    brain_observe::features::add(&store, "twin/app", "core-engine", "Engine", "building").unwrap();
    brain_observe::features::add(&store, "twin/app", "core-ui", "Interface", "building").unwrap();
    let parent = brain_observe::features::feature_sid("twin/app", "core");
    for part in ["core-engine", "core-ui"] {
        brain_observe::features::link(
            &store,
            "twin/app",
            part,
            brain_observe::features::PART_OF,
            &parent,
        )
        .unwrap();
    }
    let file = StableId::derive(&["file", "crates/core-lib/src/lib.rs"]);
    let decision = StableId::derive(&["decision", "twin/app", "adr-001-shape"]);
    let readme = StableId::derive(&["file", "README.md"]);
    for (predicate, target) in [
        ("implemented_by", &file),
        ("tested_by", &file),
        ("decided_by", &decision),
        ("documented_in", &readme),
    ] {
        brain_observe::features::link(&store, "twin/app", "core-engine", predicate, target).unwrap();
    }

    let view = f.state.read(crate::query::evidence::build).unwrap();
    let claim = view
        .claims
        .iter()
        .find(|c| c.claim.starts_with("Core is"))
        .expect("the parent still makes a claim");

    assert!(
        !claim.supported,
        "a parent with an unfinished part cannot be supported"
    );
    assert!(
        claim.verdict.contains("1 of 2 parts") && claim.verdict.contains("Interface"),
        "the verdict counts parts and names the blocker: {}",
        claim.verdict
    );

    // Both parts appear as proof, each answering for itself — the ready
    // one positively, the empty one not.
    let engine = claim
        .proof
        .iter()
        .find(|p| p.text.starts_with("Engine"))
        .expect("the ready part is proof");
    assert_eq!(engine.tone, "good");
    let ui = claim
        .proof
        .iter()
        .find(|p| p.text.starts_with("Interface"))
        .expect("the unready part is proof too");
    assert_eq!(ui.tone, "watch");
    assert!(ui.text.contains("not ready yet"), "{}", ui.text);
    assert_eq!(
        ui.basis.as_deref(),
        Some("nothing is linked to it at all"),
        "and it says why"
    );

    // The parent is judged by parts, so its own empty slots are not
    // reported as failures — that is the normal shape for a parent.
    assert!(
        !claim.proof.iter().any(|p| p.text.starts_with("nothing is linked as")),
        "a parent judged by parts is not failing for having no direct links"
    );
}

#[test]
fn the_headline_never_claims_all_clear_while_something_needs_you() {
    let f = fixture();
    let now = f.state.read(crate::query::now::build).unwrap();

    // The fixture has a drifted document, so something does need a person.
    assert!(!now.needs_you.is_empty());
    assert!(
        !now.headline.contains("Everything checks out"),
        "a cheerful lie above a list of problems: {}",
        now.headline
    );

    // Identical concerns collapse into one, counted — four rows saying the
    // same sentence are one thing that happened four times.
    for concern in &now.needs_you {
        assert!(concern.repeats >= 1);
        assert_eq!(
            concern.also.len(),
            concern.repeats.saturating_sub(1).min(4),
            "the count and the unfoldable detail must agree"
        );
    }

    // The census reads every claim in the graph, and its arithmetic holds.
    let proof = &now.proof;
    assert!(proof.total > 0, "the fixture makes claims");
    assert_eq!(
        proof.total,
        proof.groups.iter().map(|g| g.cells.len()).sum::<usize>()
    );
    assert_eq!(
        proof.proven,
        proof
            .groups
            .iter()
            .flat_map(|g| &g.cells)
            .filter(|c| c.state == "ready")
            .count(),
        "the headline number is the number of green cells, not a separate count"
    );
    assert!(proof.sentence.contains("claim"), "{}", proof.sentence);
}

/// The coverage census asks a different question from the proof census on
/// Now — whether a record is claimed at all, rather than whether a claim
/// can show its proof — so its arithmetic has to close on its own.
#[test]
fn coverage_counts_every_record_once_and_names_what_nothing_claims() {
    let f = fixture();
    let view = f.state.read(crate::query::features::build).unwrap();
    let coverage = view
        .coverage
        .expect("the fixture's feature declares links, so the question was asked");

    assert!(
        coverage.claimed > 0 && coverage.claimed < coverage.total,
        "{} of {}",
        coverage.claimed,
        coverage.total
    );
    assert_eq!(
        coverage.claimed,
        coverage.rows.iter().map(|row| row.claimed).sum::<usize>()
    );
    assert_eq!(
        coverage.total,
        coverage.rows.iter().map(|row| row.total).sum::<usize>()
    );
    for row in &coverage.rows {
        assert!(row.claimed <= row.total, "{}", row.kind);
        assert_eq!(
            row.unclaimed_total,
            row.total - row.claimed,
            "what is claimed and what is not must add up for {}",
            row.kind
        );
        // A shown list never passes for the whole list.
        assert!(row.unclaimed.len() <= row.unclaimed_total);
    }
}

/// A row that serves a feature says so, and one that serves nothing says
/// nothing — a graph is mostly unclaimed, and a label on every row would
/// be noise rather than information.
#[test]
fn a_record_names_the_features_it_serves_and_stays_quiet_otherwise() {
    let f = fixture();
    let view = f
        .state
        .read(|loaded| crate::query::library::build(loaded, "decisions", ""))
        .unwrap();
    assert!(!view.items.is_empty(), "the fixture records a decision");

    let named: Vec<&crate::dto::ShelfItem> =
        view.items.iter().filter(|item| !item.features.is_empty()).collect();
    assert!(
        !named.is_empty(),
        "the fixture's feature is decided by one of these"
    );
    for item in named {
        for reference in &item.features {
            assert_eq!(reference.kind, "feature");
            assert!(!reference.label.is_empty());
        }
    }
}
