//! Response shapes.
//!
//! Every DTO pairs the structured fact with the sentence that explains it
//! (built in [`crate::say`]), so the browser renders text it was given
//! rather than text it composed.

use serde::Serialize;

/// The graph view a response was computed from. `head` alone is not
/// enough: unbound observations and relations also advance the graph, so
/// the event cursor travels with it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Snapshot {
    pub prefix: String,
    pub head: Option<String>,
    pub cursor: usize,
    pub objects: usize,
    /// When the graph last changed (event-log mtime).
    pub changed_at_ms: u64,
    pub generated_at_ms: u64,
}

/// A pointer to something Eyes can open.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Ref {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub noun: String,
    pub glyph: String,
}

/// One judgment about one thing: what is true, and why.
#[derive(Debug, Clone, Serialize)]
pub struct Fact {
    /// "may be wrong", "no test covers it", "29 files import this"
    pub text: String,
    /// The evidence behind it, in the same voice.
    pub reason: Option<String>,
    pub tone: String,
    pub target: Option<Ref>,
}

// ---------------------------------------------------------------------------
// Now
// ---------------------------------------------------------------------------

/// Something that wants a person. Ordered worst-first.
#[derive(Debug, Clone, Serialize)]
pub struct Concern {
    /// `act` (broken or blocked), `watch` (drifting), `note` (worth knowing).
    pub severity: String,
    pub title: String,
    pub reason: String,
    /// The exact command that resolves it — Eyes never writes.
    pub fix_command: Option<String>,
    pub target: Option<Ref>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stat {
    pub label: String,
    pub value: String,
    pub note: Option<String>,
    pub tone: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SinceLastSession {
    /// False when this graph has never been consolidated (`brain sleep`).
    pub known: bool,
    pub when: Option<String>,
    pub summary: String,
    pub episodes: Vec<Episode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttentionCard {
    pub label: String,
    pub kind: String,
    pub noun: String,
    pub glyph: String,
    pub id: Option<String>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NowView {
    pub snapshot: Snapshot,
    /// The worst true thing, in one sentence.
    pub headline: String,
    pub subhead: String,
    pub needs_you: Vec<Concern>,
    pub since: SinceLastSession,
    pub attention: Vec<AttentionCard>,
    pub stats: Vec<Stat>,
}

// ---------------------------------------------------------------------------
// Library
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Shelf {
    pub id: String,
    pub label: String,
    pub note: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CoverageCell {
    pub label: String,
    pub met: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub when: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShelfItem {
    pub id: String,
    pub label: String,
    pub title: String,
    pub kind: String,
    pub noun: String,
    pub glyph: String,
    /// "current", "may be wrong", "finished", "verified"…
    pub state: Option<String>,
    pub state_note: Option<String>,
    pub tone: String,
    pub when: Option<String>,
    pub at_ms: u64,
    /// Short human facts shown under the title.
    pub facts: Vec<String>,
    /// Definition-of-done cells (features only).
    pub coverage: Option<Vec<CoverageCell>>,
    /// Run results (test protocols only).
    pub results: Option<TestSummary>,
    /// First lines of the body, for shelves you read rather than scan.
    pub excerpt: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LibraryView {
    pub snapshot: Snapshot,
    pub shelves: Vec<Shelf>,
    pub shelf: String,
    pub label: String,
    pub note: String,
    pub items: Vec<ShelfItem>,
}

/// What kinds of thing this brain knows about, and what each one promises.
#[derive(Debug, Clone, Serialize)]
pub struct Concept {
    pub kind: String,
    pub label: String,
    pub noun: String,
    pub glyph: String,
    pub purpose: String,
    pub requires: Vec<String>,
    pub home: Vec<String>,
    pub placement: String,
    pub placement_note: String,
    pub enforcement: String,
    pub enforcement_note: String,
    pub rot_note: String,
    pub count: usize,
    /// What the brain learned about how well this contract works.
    pub verdicts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConceptsView {
    pub snapshot: Snapshot,
    pub concepts: Vec<Concept>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FailingCase {
    pub name: String,
    pub id: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CaseHistory {
    pub name: String,
    pub id: String,
    pub result: String,
    pub flips: usize,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestsView {
    pub snapshot: Snapshot,
    pub headline: String,
    pub last_run: Option<TestSummary>,
    pub runs: Vec<TestSummary>,
    pub failing: Vec<FailingCase>,
    pub flaky: Vec<CaseHistory>,
    pub declared: usize,
    pub files: usize,
    pub uncovered: Vec<Ref>,
}

// ---------------------------------------------------------------------------
// Thing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Body {
    pub format: String,
    pub media_type: String,
    pub origin: String,
    pub verified: bool,
    pub path: Option<String>,
    pub size_bytes: usize,
    pub text: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Version {
    pub at_ms: u64,
    pub when: String,
    pub hash: String,
    pub note: String,
    pub current: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryEntry {
    pub at_ms: u64,
    pub when: String,
    pub text: String,
    pub source: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelationLink {
    pub phrase: String,
    pub predicate: String,
    pub outgoing: bool,
    pub other: Ref,
}

/// The local shape around one thing. Position carries meaning: `upstream`
/// is what it depends on, `downstream` is what depends on it.
#[derive(Debug, Clone, Serialize)]
pub struct Neighborhood {
    pub center: Ref,
    pub upstream: Vec<Ref>,
    pub downstream: Vec<Ref>,
    pub tests: Vec<Ref>,
    pub docs: Vec<Ref>,
    pub decisions: Vec<Ref>,
    pub upstream_total: usize,
    pub downstream_total: usize,
    pub sentence: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stage {
    pub label: String,
    pub note: String,
    pub state: String,
    pub when: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ThingExtras {
    /// Governed change: proposal → apply → verify, as reached so far.
    pub stages: Vec<Stage>,
    /// Feature: definition-of-done cells.
    pub coverage: Vec<CoverageCell>,
    /// Decision: what it replaced, and what replaced it.
    pub supersedes: Vec<Ref>,
    pub superseded_by: Vec<Ref>,
    /// Test case: pass/fail flips over time.
    pub flips: Vec<HistoryEntry>,
    /// Governed change: the recorded before/after text.
    pub before_text: Option<String>,
    pub after_text: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThingView {
    pub snapshot: Snapshot,
    pub id: String,
    pub label: String,
    pub title: String,
    pub kind: String,
    pub noun: String,
    pub glyph: String,
    pub state: Option<String>,
    pub state_note: Option<String>,
    pub tone: String,
    pub facts: Vec<Fact>,
    pub body: Option<Body>,
    pub body_error: Option<String>,
    pub neighborhood: Neighborhood,
    pub relations: Vec<RelationLink>,
    pub versions: Vec<Version>,
    pub history: Vec<HistoryEntry>,
    pub extras: ThingExtras,
    /// Machine identity, shown only under a details disclosure.
    pub details: Vec<(String, String)>,
}

// ---------------------------------------------------------------------------
// Map
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct MapBlock {
    pub id: String,
    pub label: String,
    pub path: String,
    pub files: usize,
    pub symbols: usize,
    /// Dependency layer: 0 is depended on by everything above it.
    pub layer: usize,
    /// 0..=100, meaning set by the lens.
    pub value: u32,
    pub tone: String,
    pub sentence: String,
    pub facts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MapEdge {
    pub from: String,
    pub to: String,
    pub weight: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct MapView {
    pub snapshot: Snapshot,
    pub lens: String,
    pub lens_label: String,
    pub lens_note: String,
    pub lenses: Vec<(String, String)>,
    pub blocks: Vec<MapBlock>,
    pub edges: Vec<MapEdge>,
    pub sentence: String,
}

// ---------------------------------------------------------------------------
// Timeline
// ---------------------------------------------------------------------------

/// A batch of graph activity that happened together — one refresh, one
/// test run, one governed change, one consolidation.
#[derive(Debug, Clone, Serialize)]
pub struct Episode {
    pub at_ms: u64,
    pub when: String,
    pub kind: String,
    pub title: String,
    pub facts: Vec<String>,
    pub items: Vec<Ref>,
    pub more: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimelineView {
    pub snapshot: Snapshot,
    pub episodes: Vec<Episode>,
}

// ---------------------------------------------------------------------------
// Find
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct FindHit {
    pub target: Ref,
    /// Why this matched, in the same voice as everything else.
    pub because: String,
    pub state: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FindView {
    pub snapshot: Snapshot,
    pub query: String,
    pub hits: Vec<FindHit>,
    pub note: Option<String>,
}
