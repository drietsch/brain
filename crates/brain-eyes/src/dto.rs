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
    /// Every recorded run, in full.
    pub protocols: Vec<Protocol>,
    /// Every recorded case, with its verdict and its evidence.
    pub cases: Vec<CaseRow>,
    /// The test files the twin classified, by framework.
    pub suites: Vec<Suite>,
    pub frameworks: Vec<FrameworkCount>,
}

/// One imported run, as a thing you can open.
#[derive(Debug, Clone, Serialize)]
pub struct Protocol {
    pub id: String,
    pub when: String,
    pub at_ms: u64,
    pub verdict: String,
    pub tone: String,
    pub source: String,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub duration: Option<String>,
    /// Cases this run named: what failed, what was skipped, what changed
    /// its mind. Passing cases are not linked to runs — see ADR-027.
    pub named: Vec<CaseRef>,
    /// The Evidence object this run wrote, if any.
    pub evidence: Option<String>,
    pub verified_change: Option<Ref>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CaseRef {
    pub id: String,
    pub name: String,
    pub result: String,
    pub tone: String,
}

/// One test case, everything the graph knows about it.
#[derive(Debug, Clone, Serialize)]
pub struct CaseRow {
    pub id: String,
    pub name: String,
    /// The suite or file it belongs to, for grouping.
    pub group: String,
    pub result: String,
    pub tone: String,
    pub when: String,
    pub at_ms: u64,
    pub framework: Option<String>,
    pub duration: Option<String>,
    pub error: Option<String>,
    pub retries: usize,
    pub flips: usize,
    pub note: Option<String>,
    pub file: Option<Ref>,
    /// Screenshots, recordings and traces the run produced.
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Attachment {
    pub id: String,
    pub label: String,
    pub noun: String,
    pub subtype: String,
    pub path: String,
}

/// A file the twin classified as holding tests.
#[derive(Debug, Clone, Serialize)]
pub struct Suite {
    pub id: String,
    pub path: String,
    pub framework: String,
    pub framework_label: String,
    pub declared: usize,
    pub whole_file: bool,
    pub covers: Vec<Ref>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FrameworkCount {
    pub framework: String,
    pub label: String,
    pub files: usize,
    pub declared: usize,
}

// ---------------------------------------------------------------------------
// Work
// ---------------------------------------------------------------------------

/// An agent session: the graph's only record of who did something.
#[derive(Debug, Clone, Serialize)]
pub struct Session {
    pub id: String,
    pub agent: String,
    pub agent_label: String,
    pub objective: String,
    pub model: Option<String>,
    pub when: String,
    pub at_ms: u64,
    pub ran_for: String,
    pub turns: usize,
    pub tools: Vec<ToolUse>,
    pub live: bool,
    pub state: String,
    pub touched: Vec<Ref>,
    pub more_touched: usize,
    /// Artifacts this session produced, derived from what it edited.
    pub produced: Vec<Ref>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolUse {
    pub name: String,
    pub label: String,
    pub count: usize,
}

/// Something in flight that is not a session: a change, a plan.
#[derive(Debug, Clone, Serialize)]
pub struct WorkItem {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub noun: String,
    pub glyph: String,
    pub stage: String,
    pub note: String,
    pub tone: String,
    pub when: String,
    pub at_ms: u64,
    pub fix_command: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkView {
    pub snapshot: Snapshot,
    pub headline: String,
    pub sessions: Vec<Session>,
    pub changes: Vec<WorkItem>,
    pub plans: Vec<WorkItem>,
    /// Told plainly when no agent history has been imported. The
    /// sentence and the command are separate fields, as everywhere else:
    /// a command is not prose, and mixing them is how flag names end up
    /// being read out loud.
    pub sessions_hint: Option<String>,
    pub sessions_hint_command: Option<String>,
}

// ---------------------------------------------------------------------------
// Evidence
// ---------------------------------------------------------------------------

/// A claim the system makes, and what stands behind it.
#[derive(Debug, Clone, Serialize)]
pub struct Claim {
    pub id: String,
    pub subject: Option<Ref>,
    /// What is being asserted.
    pub claim: String,
    /// Whether the proof holds it up.
    pub supported: bool,
    pub tone: String,
    /// Why it is or is not supported.
    pub verdict: String,
    pub proof: Vec<Proof>,
    pub category: String,
    pub fix_command: Option<String>,
}

/// One link in a proof: a fact, and what kind of fact it is.
#[derive(Debug, Clone, Serialize)]
pub struct Proof {
    pub text: String,
    /// What this level of verification actually establishes.
    pub basis: Option<String>,
    pub tone: String,
    pub target: Option<Ref>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvidenceCategory {
    pub id: String,
    pub label: String,
    pub note: String,
    pub supported: usize,
    pub unsupported: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvidenceView {
    pub snapshot: Snapshot,
    pub headline: String,
    pub categories: Vec<EvidenceCategory>,
    pub claims: Vec<Claim>,
}

// ---------------------------------------------------------------------------
// Media
// ---------------------------------------------------------------------------

/// A picture, recording or audio file the graph knows about.
#[derive(Debug, Clone, Serialize)]
pub struct MediaItem {
    pub id: String,
    pub label: String,
    pub subtype: String,
    pub noun: String,
    pub path: String,
    /// The command that produced it, when one was recorded.
    pub rendered_from: Option<String>,
    pub when: String,
    pub at_ms: u64,
    pub state: String,
    pub state_note: String,
    pub tone: String,
    pub owner: Option<Ref>,
    pub depicts: Vec<Ref>,
}

/// One chapter of the generated tour.
#[derive(Debug, Clone, Serialize)]
pub struct Chapter {
    pub id: String,
    pub title: String,
    pub command: String,
    pub image: Option<MediaItem>,
    /// The narration sentence that belongs to this chapter, when the
    /// script has one for it.
    pub narration: Option<String>,
}

/// A sentence the recorded tour still asserts that the graph has moved on
/// from.
#[derive(Debug, Clone, Serialize)]
pub struct NarrationDrift {
    pub recorded: Option<String>,
    pub current: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Tour {
    pub video: Option<MediaItem>,
    pub script: Vec<String>,
    pub chapters: Vec<Chapter>,
    pub state: String,
    pub state_note: String,
    pub tone: String,
    pub drift: Vec<NarrationDrift>,
    pub regenerate_command: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MediaView {
    pub snapshot: Snapshot,
    pub headline: String,
    pub tour: Option<Tour>,
    pub items: Vec<MediaItem>,
}

// ---------------------------------------------------------------------------
// MRI — the living graph
// ---------------------------------------------------------------------------

/// One thing in the anatomy.
///
/// Positions are computed on the server and are stable for a given graph
/// version, so the picture does not rearrange itself while a person is
/// looking at it. `level` drives level-of-detail: nothing is ever dropped,
/// detail resolves as the camera approaches.
#[derive(Debug, Clone, Serialize)]
pub struct MriNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub glyph: String,
    pub cluster: String,
    pub group: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub size: f32,
    /// 0 landmarks, 1 ordinary things, 2 fine detail.
    pub level: u8,
    pub tone: String,
    /// Why it is lit: "changed", "failing", "working", "unfinished".
    pub pulse: Option<String>,
    pub at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MriEdge {
    /// Indices into `nodes`, not identifiers: the browser draws millions
    /// of these and should not parse a string per line.
    pub a: u32,
    pub b: u32,
    pub predicate: String,
    /// The higher of the two endpoints' levels.
    pub level: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct MriCluster {
    pub id: String,
    pub label: String,
    pub note: String,
    pub count: usize,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub radius: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct MriView {
    pub snapshot: Snapshot,
    pub headline: String,
    pub clusters: Vec<MriCluster>,
    pub nodes: Vec<MriNode>,
    pub edges: Vec<MriEdge>,
    /// Counts per level, so the view can say how much is on screen.
    pub levels: Vec<usize>,
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
    /// Governed change: the audit record, in the order it happened.
    pub audit: Vec<AuditEntry>,
    /// Test case: attachments the run produced about it.
    pub attachments: Vec<Attachment>,
    /// Agent session: what it did.
    pub session: Option<Session>,
    /// Feature: its parts, its strip, and what is holding it up.
    pub feature: Option<FeatureNode>,
}

/// One line of an action's story. `recorded` distinguishes what the graph
/// holds from what Eyes worked out — a reconstructed command is useful,
/// but presenting it as a record would be a lie.
#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub label: String,
    pub value: String,
    pub note: Option<String>,
    pub recorded: bool,
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

// ---------------------------------------------------------------------------
// Features and their parts
// ---------------------------------------------------------------------------

/// One cell of the dimension strip: a part of a feature, or — for a
/// feature with no parts — one of its requirements.
///
/// The same five states carry through every scale at which the strip is
/// drawn, and each is a shape as well as a colour.
#[derive(Debug, Clone, Serialize)]
pub struct StripCell {
    pub label: String,
    /// ready | stale | failing | absent | unproven
    pub state: String,
    /// What that means, in a sentence.
    pub detail: String,
    /// The part this cell stands for, when it is a part.
    pub id: Option<String>,
}

/// A feature and everything under it.
#[derive(Debug, Clone, Serialize)]
pub struct FeatureNode {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub status: String,
    pub done: bool,
    pub met: usize,
    pub total: usize,
    /// Whether it is judged by its parts rather than its own requirements.
    pub by_parts: bool,
    pub blocked_by: Option<String>,
    pub verdict: String,
    pub tone: String,
    pub strip: Vec<StripCell>,
    pub parts: Vec<FeatureNode>,
    pub depth: usize,
    pub when: String,
    pub at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeaturesView {
    pub snapshot: Snapshot,
    pub headline: String,
    pub note: String,
    pub roots: Vec<FeatureNode>,
    /// Every dimension name in use, so the list can be faceted by them.
    pub dimensions: Vec<String>,
}
