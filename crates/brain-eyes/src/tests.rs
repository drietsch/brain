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

    // The code moves after the documents were written: drift.
    std::thread::sleep(std::time::Duration::from_millis(5));
    fs::write(
        root.join("crates/core-lib/src/lib.rs"),
        "pub fn core_thing() { /* changed */ }\n",
    )
    .unwrap();
    twin::refresh(&store, root, "twin/app").unwrap();

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
fn tests_view_reports_results_not_just_declarations() {
    let f = fixture();
    let view = f.state.read(crate::query::library::tests).unwrap();
    assert!(
        view.headline.contains("No test run"),
        "honest when nothing was imported: {}",
        view.headline
    );
    assert!(view.last_run.is_none());
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
    for stat in &now.stats {
        prose.push(stat.label.clone());
        prose.push(stat.value.clone());
        prose.extend(stat.note.clone());
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
