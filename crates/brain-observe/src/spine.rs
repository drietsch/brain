//! The spine: which feature each thing in the graph serves.
//!
//! A feature declares its files (ADR-028: a claim someone made, never
//! inferred). Everything else in this graph already points at files, and
//! does so automatically — a test file `covers` the sources it imports, a
//! case is `defined_in` one, a document `mentions` the paths it names, a
//! session `touched` what it edited, a governed change `changes` its
//! target. So the file is the join, and a feature inherits its tests, its
//! documents, its sessions and its changes by derivation over edges that
//! already exist.
//!
//! Two rules keep the derivation honest:
//!
//! 1. **Nothing here invents a feature or a part.** It computes
//!    *attribution* — which existing feature an existing entity serves.
//!    That is a different act from inferring structure from a directory
//!    name, which ADR-028 forbids.
//! 2. **A derived claim names the file that carries it.** Every `Reached`
//!    that was not declared outright records the file it came through, so
//!    the attribution can be checked rather than believed.
//!
//! The walk stops at two hops. Following `imports` transitively would let
//! a feature that declares one file claim the entire dependency cone,
//! and the spine would smear into a single blur.

use crate::features;
use crate::twin::{live_from, live_to};
use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::{Index, MemIndex};
use brain_store::{Store, StoreError};
use std::collections::{BTreeMap, BTreeSet};

/// Predicates that point *at* a declared source file from something that
/// is not a feature. Each is read backwards, from the file to whatever
/// names it.
const INBOUND: &[&str] = &[
    "covers",      // test file      -> source file
    "defined_in",  // test case      -> source file
    "mentions",    // any artifact   -> source file
    "recorded_in", // any artifact   -> the file it lives in
    "touched",     // agent session  -> source file
    "changes",     // governed change-> source file
];

/// What counts as reaching the file a *declared document* happens to live
/// in — as opposed to the code the feature declares.
///
/// Only work done on that file counts. A second document that merely
/// mentions this one is not part of the feature: following `mentions`
/// here attributed the README, the roadmap and the architecture note to
/// whichever feature declared `docs/twin.md`, which is a smear rather
/// than an attribution.
const INBOUND_ARTIFACT: &[&str] = &[
    "touched", // agent session   -> the document's file
    "changes", // governed change -> the document's file
];

/// Kinds a feature can meaningfully be said to claim.
///
/// `symbol` is excluded because a symbol is claimed exactly when its file
/// is — 1159 of them would drown the census while saying nothing new.
/// `module` is an unresolved external import and belongs to nobody;
/// `test_run` is an event rather than a deliverable and is attributed
/// through the cases it named.
pub const CLAIMABLE: &[&str] = &[
    "source_file",
    "test_case",
    "decision",
    "doc",
    "runbook",
    "plan",
    "task_list",
    "agent_session",
    "change",
    "asset",
    "skill",
    "agent_config",
];

/// How a feature reached something. The order is precedence: a shorter
/// path wins, so an entity declared outright is never reported as merely
/// reached through a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Via {
    /// The feature itself links to it.
    Declared,
    /// One of its parts links to it.
    Part,
    /// It points at a file the feature declares.
    File,
    /// It is a case inside a test file that covers a declared file.
    Suite,
}

impl Via {
    pub fn as_str(self) -> &'static str {
        match self {
            Via::Declared => "declared",
            Via::Part => "part",
            Via::File => "file",
            Via::Suite => "suite",
        }
    }
}

/// One entity a feature reaches, and the path that says so.
#[derive(Debug, Clone)]
pub struct Reached {
    pub sid: StableId,
    pub kind: String,
    pub via: Via,
    /// The declared file the derivation passed through. Always set for
    /// `File` and `Suite` — a derived claim that cannot name its join is
    /// an invention, and this is the field that makes it checkable.
    pub through: Option<StableId>,
    /// The predicate of the last hop.
    pub predicate: String,
}

/// A feature that claims some entity, and why.
#[derive(Debug, Clone)]
pub struct Owned {
    pub feature: StableId,
    pub slug: String,
    pub title: String,
    pub via: Via,
    pub through: Option<StableId>,
    pub predicate: String,
}

/// Everything one feature reaches.
#[derive(Debug, Clone)]
pub struct FeatureReach {
    pub feature: StableId,
    pub slug: String,
    pub title: String,
    /// The definition-of-done targets exactly as declared. The *claim*.
    pub declared: BTreeMap<String, Vec<StableId>>,
    /// Source files this feature or any of its parts declares. The join
    /// frontier, and what every derived claim is measured against.
    pub files: BTreeSet<StableId>,
    /// Files its declared documents are recorded in. A narrower join:
    /// only work done *on* those files counts (see `INBOUND_ARTIFACT`).
    pub artifact_files: BTreeSet<StableId>,
    /// Everything reached, by entity kind. The *derivation*.
    pub by_kind: BTreeMap<String, Vec<Reached>>,
}

impl FeatureReach {
    pub fn of_kind(&self, kind: &str) -> &[Reached] {
        self.by_kind.get(kind).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// A declared slot nothing observed corroborates.
///
/// The definition of done counts links. This asks the stronger question:
/// does anything the twin recorded *by itself* agree? A document claimed
/// as a feature's documentation that mentions none of its files is a
/// claim, not evidence.
#[derive(Debug, Clone)]
pub struct Uncorroborated {
    pub slug: String,
    pub predicate: String,
    /// The declared targets of this slot that nothing corroborates. One
    /// row per slot, not per target: six files failing the same check is
    /// one thing to know about, six times.
    pub targets: Vec<StableId>,
    /// What was looked for and not found, in a person's words.
    pub why: String,
}

/// How much of one kind any feature claims.
#[derive(Debug, Clone)]
pub struct KindCoverage {
    pub kind: String,
    pub claimed: usize,
    pub total: usize,
}

/// Which feature everything serves. A pure function of the graph, so it
/// is built once per version and shared.
#[derive(Debug, Default)]
pub struct Spine {
    reach: BTreeMap<String, FeatureReach>,
    owners: BTreeMap<StableId, Vec<Owned>>,
    census: Vec<KindCoverage>,
    unclaimed: BTreeMap<String, Vec<StableId>>,
    uncorroborated: Vec<Uncorroborated>,
    asked: bool,
}

impl Spine {
    pub fn reach(&self, slug: &str) -> Option<&FeatureReach> {
        self.reach.get(slug)
    }

    /// The features that claim this entity, nearest claim first. Empty is
    /// the ordinary answer and renders as nothing.
    pub fn features_of(&self, sid: &StableId) -> &[Owned] {
        self.owners.get(sid).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn census(&self) -> &[KindCoverage] {
        &self.census
    }

    pub fn unclaimed(&self, kind: &str) -> &[StableId] {
        self.unclaimed.get(kind).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn uncorroborated(&self) -> &[Uncorroborated] {
        &self.uncorroborated
    }

    /// Claimed and total across every claimable kind.
    pub fn claimed_total(&self) -> (usize, usize) {
        self.census.iter().fold((0, 0), |(c, t), row| {
            (c + row.claimed, t + row.total)
        })
    }

    /// Whether any feature declares anything at all. When false, every
    /// readout above stays silent: the question was never asked, and a
    /// coverage report on a graph with no spine is noise.
    pub fn asked(&self) -> bool {
        self.asked
    }

    pub fn slugs(&self) -> impl Iterator<Item = &str> {
        self.reach.keys().map(String::as_str)
    }
}

/// Build the spine for one prefix.
pub fn build(store: &Store, index: &MemIndex, prefix: &str) -> Result<Spine, StoreError> {
    let mut spine = Spine::default();
    let dod = features::dod(store, index)?;

    // Pass 1 — what every feature declares, and what its parts declare.
    let rows = features::list(store, index, prefix)?;
    for row in &rows {
        let sid = features::feature_sid(prefix, &row.slug);
        let mut reach = FeatureReach {
            feature: sid.clone(),
            slug: row.slug.clone(),
            title: row.title.clone(),
            declared: BTreeMap::new(),
            files: BTreeSet::new(),
            artifact_files: BTreeSet::new(),
            by_kind: BTreeMap::new(),
        };
        for predicate in &dod {
            let mut targets = Vec::new();
            for (_, to) in live_from(index, store, &sid, predicate)? {
                if !targets.contains(&to) {
                    targets.push(to);
                }
            }
            if !targets.is_empty() {
                spine.asked = true;
                reach.declared.insert(predicate.clone(), targets);
            }
        }
        spine.reach.insert(row.slug.clone(), reach);
    }

    // Pass 2 — the frontier. A feature owns what it declares directly and
    // what its descendants declare, so a parent answers for its whole
    // subtree without re-linking anything.
    let mut descendants: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in &rows {
        let sid = features::feature_sid(prefix, &row.slug);
        descendants.insert(row.slug.clone(), subtree(store, index, &sid)?);
    }

    for row in &rows {
        let own: Vec<(String, StableId)> = spine
            .reach
            .get(&row.slug)
            .map(|r| {
                r.declared
                    .iter()
                    .flat_map(|(p, ts)| ts.iter().map(move |t| (p.clone(), t.clone())))
                    .collect()
            })
            .unwrap_or_default();
        let mut claims: Vec<(Via, String, StableId)> = own
            .into_iter()
            .map(|(p, t)| (Via::Declared, p, t))
            .collect();
        for child in descendants.get(&row.slug).into_iter().flatten() {
            if let Some(childreach) = spine.reach.get(child) {
                for (predicate, targets) in &childreach.declared {
                    for target in targets {
                        claims.push((Via::Part, predicate.clone(), target.clone()));
                    }
                }
            }
        }

        let mut files = BTreeSet::new();
        let mut artifact_files = BTreeSet::new();
        let mut by_kind: BTreeMap<String, Vec<Reached>> = BTreeMap::new();
        for (via, predicate, target) in claims {
            let kind = kind_of(store, index, &target);
            if kind == "source_file" {
                files.insert(target.clone());
            } else {
                // A declared document lives in a file. That file joins the
                // feature to whoever edited it — and to nothing else.
                for (_, file) in live_from(index, store, &target, "recorded_in")? {
                    artifact_files.insert(file);
                }
            }
            if !kind.is_empty() {
                push_reached(
                    by_kind.entry(kind.clone()).or_default(),
                    Reached {
                        sid: target,
                        kind,
                        via,
                        through: None,
                        predicate,
                    },
                );
            }
        }
        if let Some(reach) = spine.reach.get_mut(&row.slug) {
            // A file that is both declared code and a document's home is
            // code: the wider join wins.
            artifact_files.retain(|file| !files.contains(file));
            reach.files = files;
            reach.artifact_files = artifact_files;
            reach.by_kind = by_kind;
        }
    }

    // Pass 3 — the inverse, over each distinct frontier file exactly once.
    // A file serving two features costs one lookup, not two.
    let mut all_files: BTreeSet<StableId> = BTreeSet::new();
    let mut all_artifact_files: BTreeSet<StableId> = BTreeSet::new();
    for reach in spine.reach.values() {
        all_files.extend(reach.files.iter().cloned());
        all_artifact_files.extend(reach.artifact_files.iter().cloned());
    }
    all_artifact_files.retain(|file| !all_files.contains(file));

    let mut found: BTreeMap<StableId, Vec<(StableId, String, String, bool)>> = BTreeMap::new();
    for (file, predicates) in all_files
        .iter()
        .map(|f| (f, INBOUND))
        .chain(all_artifact_files.iter().map(|f| (f, INBOUND_ARTIFACT)))
    {
        let mut hits: Vec<(StableId, String, String, bool)> = Vec::new();
        for predicate in predicates {
            for (_, from) in live_to(index, store, file, predicate)? {
                let kind = kind_of(store, index, &from);
                if kind.is_empty() {
                    continue;
                }
                // Second hop, and the only one: the cases inside a test
                // file that covers a declared file.
                if *predicate == "covers" {
                    for (_, case) in live_to(index, store, &from, "defined_in")? {
                        let case_kind = kind_of(store, index, &case);
                        if !case_kind.is_empty() {
                            hits.push((case, case_kind, "defined_in".to_string(), true));
                        }
                    }
                }
                hits.push((from, kind, (*predicate).to_string(), false));
            }
        }
        found.insert(file.clone(), hits);
    }

    for reach in spine.reach.values_mut() {
        let frontier: Vec<StableId> = reach
            .files
            .iter()
            .chain(reach.artifact_files.iter())
            .cloned()
            .collect();
        for file in &frontier {
            for (sid, kind, predicate, second_hop) in found.get(file).into_iter().flatten() {
                if sid == &reach.feature {
                    continue;
                }
                push_reached(
                    reach.by_kind.entry(kind.clone()).or_default(),
                    Reached {
                        sid: sid.clone(),
                        kind: kind.clone(),
                        via: if *second_hop { Via::Suite } else { Via::File },
                        through: Some(file.clone()),
                        predicate: predicate.clone(),
                    },
                );
            }
        }
    }

    // Pass 4 — the inverse index, the census, and what nothing corroborates.
    for reach in spine.reach.values() {
        for reached in reach.by_kind.values().flatten() {
            let owners = spine.owners.entry(reached.sid.clone()).or_default();
            if let Some(existing) = owners.iter_mut().find(|o| o.feature == reach.feature) {
                if reached.via < existing.via {
                    existing.via = reached.via;
                    existing.through = reached.through.clone();
                    existing.predicate = reached.predicate.clone();
                }
                continue;
            }
            owners.push(Owned {
                feature: reach.feature.clone(),
                slug: reach.slug.clone(),
                title: reach.title.clone(),
                via: reached.via,
                through: reached.through.clone(),
                predicate: reached.predicate.clone(),
            });
        }
    }
    for owners in spine.owners.values_mut() {
        owners.sort_by(|a, b| a.via.cmp(&b.via).then(a.slug.cmp(&b.slug)));
    }

    if spine.asked {
        spine.census = census(store, index, prefix, &spine.owners, &mut spine.unclaimed)?;
        spine.uncorroborated = uncorroborated(store, index, &spine.reach)?;
    }
    Ok(spine)
}

/// Keep the strongest path to a given entity, never a duplicate row.
fn push_reached(into: &mut Vec<Reached>, candidate: Reached) {
    if let Some(existing) = into.iter_mut().find(|r| r.sid == candidate.sid) {
        if candidate.via < existing.via {
            *existing = candidate;
        }
        return;
    }
    into.push(candidate);
}

/// Every feature below this one, by slug, cycle-safe and depth-capped —
/// the same guard `features::evaluate` uses, for the same reason.
fn subtree(store: &Store, index: &MemIndex, sid: &StableId) -> Result<Vec<String>, StoreError> {
    let mut out = Vec::new();
    let mut seen: BTreeSet<StableId> = BTreeSet::from([sid.clone()]);
    let mut frontier = vec![(sid.clone(), 0usize)];
    while let Some((current, depth)) = frontier.pop() {
        if depth >= features::MAX_DEPTH {
            continue;
        }
        for (child, slug) in features::children(store, index, &current)? {
            if !seen.insert(child.clone()) {
                continue;
            }
            out.push(slug);
            frontier.push((child, depth + 1));
        }
    }
    Ok(out)
}

fn census(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    owners: &BTreeMap<StableId, Vec<Owned>>,
    unclaimed: &mut BTreeMap<String, Vec<StableId>>,
) -> Result<Vec<KindCoverage>, StoreError> {
    let mut out = Vec::new();
    for kind in CLAIMABLE {
        let mut claimed = 0usize;
        let mut total = 0usize;
        let mut missing = Vec::new();
        let mut seen: BTreeSet<StableId> = BTreeSet::new();
        for node in index.entities_by_kind(kind) {
            let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
                continue;
            };
            // Files are keyed by path and carry no prefix label; every
            // other kind is scoped.
            if *kind != "source_file" && labels.get("prefix").map(String::as_str) != Some(prefix) {
                continue;
            }
            if !seen.insert(id.clone()) {
                continue;
            }
            if *kind == "source_file"
                && crate::twin::latest(index, store, &id, "present")?.as_deref() == Some("false")
            {
                continue; // a deleted file cannot be unclaimed; it is gone
            }
            total += 1;
            if owners.contains_key(&id) {
                claimed += 1;
            } else {
                missing.push(id);
            }
        }
        if total > 0 {
            unclaimed.insert((*kind).to_string(), missing);
            out.push(KindCoverage {
                kind: (*kind).to_string(),
                claimed,
                total,
            });
        }
    }
    Ok(out)
}

/// Declared slots that nothing the twin observed by itself supports.
fn uncorroborated(
    store: &Store,
    index: &MemIndex,
    reach: &BTreeMap<String, FeatureReach>,
) -> Result<Vec<Uncorroborated>, StoreError> {
    let mut out = Vec::new();
    for feature in reach.values() {
        for (predicate, targets) in &feature.declared {
            let mut failed = Vec::new();
            let mut why = String::new();
            for target in targets {
                let kind = kind_of(store, index, target);
                let miss = match (predicate.as_str(), kind.as_str()) {
                    // A file claimed as the tests has to be tested by
                    // something: a test file that covers it, a case
                    // defined in it, or the tests it declares itself —
                    // which is how an inline `#[cfg(test)]` module reads.
                    ("tested_by", "source_file") => {
                        let declared = crate::twin::latest(index, store, target, "tests_declared")?
                            .and_then(|v| v.parse::<usize>().ok())
                            .unwrap_or(0);
                        let covered = declared > 0
                            || !live_to(index, store, target, "covers")?.is_empty()
                            || !live_to(index, store, target, "defined_in")?.is_empty();
                        (!covered).then(|| {
                            "it declares no tests, nothing covers it, and no case is defined in it"
                                .to_string()
                        })
                    }
                    // A document or decision claimed by a feature should
                    // name at least one of the files that feature declares.
                    ("documented_in" | "decided_by", k) if k != "source_file" => {
                        let mut names_one = false;
                        for (_, file) in live_from(index, store, target, "mentions")? {
                            if feature.files.contains(&file) {
                                names_one = true;
                                break;
                            }
                        }
                        (!names_one).then(|| {
                            "it names none of the files this feature declares".to_string()
                        })
                    }
                    _ => None,
                };
                if let Some(miss) = miss {
                    why = miss;
                    failed.push(target.clone());
                }
            }
            if !failed.is_empty() {
                out.push(Uncorroborated {
                    slug: feature.slug.clone(),
                    predicate: predicate.clone(),
                    targets: failed,
                    why,
                });
            }
        }
    }
    Ok(out)
}

fn kind_of(store: &Store, index: &MemIndex, sid: &StableId) -> String {
    for node in index.entity_nodes(sid) {
        if let Ok(Object::Entity { entity_kind, .. }) = store.get(&node) {
            return entity_kind;
        }
    }
    String::new()
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

    /// A workspace where the automatic edges exist: a test file that
    /// imports the source it covers, and a document that names it.
    fn world() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn thing() {}\n").unwrap();
        std::fs::write(
            dir.path().join("src/lib.test.js"),
            "import { thing } from './lib';\ntest('works', () => {});\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("docs")).unwrap();
        std::fs::write(
            dir.path().join("docs/guide.md"),
            "# Guide\n\nHow src/lib.rs works.\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("docs/elsewhere.md"),
            "# Elsewhere\n\nNothing to do with the code.\n",
        )
        .unwrap();
        let store = Store::open(&dir.path().join(".brain")).unwrap();
        crate::templates::seed(&store).unwrap();
        crate::twin::refresh(&store, dir.path(), "twin/app").unwrap();
        (dir, store)
    }

    #[test]
    fn a_feature_reaches_what_points_at_the_files_it_declares() {
        let (_dir, store) = world();
        features::add(&store, "twin/app", "core", "Core", "building").unwrap();
        let lib = StableId::derive(&["file", "src/lib.rs"]);
        features::link(&store, "twin/app", "core", "implemented_by", &lib).unwrap();

        let index = fresh_index(&store);
        let spine = build(&store, &index, "twin/app").unwrap();
        let reach = spine.reach("core").expect("the feature is in the spine");

        assert!(reach.files.contains(&lib), "the declared file is the join");

        // The test file covers src/lib.rs because it imports it. Nobody
        // linked it to the feature; the twin recorded the edge already.
        let covering = reach.of_kind("source_file");
        let test_file = StableId::derive(&["file", "src/lib.test.js"]);
        let found = covering.iter().find(|r| r.sid == test_file).expect(
            "the covering test file is reached through the file the feature declares",
        );
        assert_eq!(found.via, Via::File);
        assert_eq!(
            found.through.as_ref(),
            Some(&lib),
            "a derived claim always names the file that carries it"
        );

        // And the document that mentions it, likewise.
        let docs = reach.of_kind("doc");
        assert!(
            docs.iter().any(|r| r.through.as_ref() == Some(&lib)),
            "the document that names the file is reached through it"
        );
    }

    #[test]
    fn the_inverse_says_which_features_a_thing_serves() {
        let (_dir, store) = world();
        features::add(&store, "twin/app", "core", "Core", "building").unwrap();
        let lib = StableId::derive(&["file", "src/lib.rs"]);
        features::link(&store, "twin/app", "core", "implemented_by", &lib).unwrap();

        let index = fresh_index(&store);
        let spine = build(&store, &index, "twin/app").unwrap();

        let owners = spine.features_of(&lib);
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].slug, "core");
        assert_eq!(owners[0].via, Via::Declared);

        // Nothing claims the unrelated document, and that is reported as
        // silence rather than as an owner.
        let elsewhere = StableId::derive(&["file", "docs/elsewhere.md"]);
        assert!(spine.features_of(&elsewhere).is_empty());
    }

    #[test]
    fn a_parent_answers_for_what_its_parts_declare() {
        let (_dir, store) = world();
        features::add(&store, "twin/app", "whole", "Whole", "building").unwrap();
        features::add(&store, "twin/app", "part", "Part", "building").unwrap();
        let parent = features::feature_sid("twin/app", "whole");
        features::link(&store, "twin/app", "part", "part_of", &parent).unwrap();
        let lib = StableId::derive(&["file", "src/lib.rs"]);
        features::link(&store, "twin/app", "part", "implemented_by", &lib).unwrap();

        let index = fresh_index(&store);
        let spine = build(&store, &index, "twin/app").unwrap();

        let whole = spine.reach("whole").unwrap();
        assert!(
            whole.files.contains(&lib),
            "a parent reaches through the files its parts declare"
        );
        let owners = spine.features_of(&lib);
        assert_eq!(owners.len(), 2, "the part and the whole both claim it");
        assert_eq!(owners[0].slug, "part", "the nearest claim comes first");
        assert_eq!(owners[0].via, Via::Declared);
        assert_eq!(owners[1].via, Via::Part);
    }

    #[test]
    fn a_document_that_names_none_of_the_files_is_not_corroborated() {
        let (_dir, store) = world();
        features::add(&store, "twin/app", "core", "Core", "building").unwrap();
        let lib = StableId::derive(&["file", "src/lib.rs"]);
        features::link(&store, "twin/app", "core", "implemented_by", &lib).unwrap();
        // Claim a document that mentions nothing this feature declares.
        let elsewhere = StableId::derive(&["doc", "twin/app", "elsewhere"]);
        features::link(&store, "twin/app", "core", "documented_in", &elsewhere).unwrap();
        // …and one that does.
        let guide = StableId::derive(&["doc", "twin/app", "guide"]);
        features::link(&store, "twin/app", "core", "documented_in", &guide).unwrap();

        let index = fresh_index(&store);
        let spine = build(&store, &index, "twin/app").unwrap();

        let flagged: Vec<&Uncorroborated> = spine
            .uncorroborated()
            .iter()
            .filter(|u| u.slug == "core")
            .collect();
        assert_eq!(flagged.len(), 1, "one row for the slot, not one per target");
        assert_eq!(flagged[0].predicate, "documented_in");
        assert_eq!(
            flagged[0].targets,
            vec![elsewhere],
            "only the document that names nothing is flagged"
        );
    }

    #[test]
    fn a_graph_with_no_feature_links_says_nothing_about_coverage() {
        let (_dir, store) = world();
        features::add(&store, "twin/app", "core", "Core", "building").unwrap();

        let index = fresh_index(&store);
        let spine = build(&store, &index, "twin/app").unwrap();
        assert!(!spine.asked(), "nothing is declared, so nothing is claimed");
        assert!(spine.census().is_empty());
        assert_eq!(spine.claimed_total(), (0, 0));
    }

    #[test]
    fn the_census_counts_every_claimable_record_exactly_once() {
        let (_dir, store) = world();
        features::add(&store, "twin/app", "core", "Core", "building").unwrap();
        let lib = StableId::derive(&["file", "src/lib.rs"]);
        features::link(&store, "twin/app", "core", "implemented_by", &lib).unwrap();

        let index = fresh_index(&store);
        let spine = build(&store, &index, "twin/app").unwrap();
        assert!(spine.asked());

        for row in spine.census() {
            assert!(row.claimed <= row.total);
            assert_eq!(
                row.total - row.claimed,
                spine.unclaimed(&row.kind).len(),
                "what is unclaimed and what is claimed must add up for {}",
                row.kind
            );
        }
        let (claimed, total) = spine.claimed_total();
        assert!(claimed > 0 && claimed < total, "{claimed} of {total}");
    }
}
