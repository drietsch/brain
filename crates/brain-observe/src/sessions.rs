//! Agent sessions: the first actor the graph has ever had.
//!
//! Everything else the brain records answers *what* changed. Nothing has
//! answered *who did it* — intents, receipts and observations carry a
//! mechanism name (`twin`, `govern`, `testrun`), never a principal. In a
//! workspace where most edits come from coding agents, that is the
//! largest missing fact.
//!
//! Claude Code and Codex both keep an append-only transcript per session.
//! This module reads those transcripts and records what a session *was* —
//! its objective, when it ran, which model, how many turns, which tools,
//! and which files it edited — as an `agent_session` entity.
//!
//! **What is deliberately not recorded.** Prompt and response bodies
//! never enter the graph. A transcript is a private working record; the
//! graph gets the truncated first instruction (so a session is
//! identifiable), tool *names* without their arguments, the paths of
//! files that were edited, and timings. Nothing else. A test asserts it.
//!
//! Scoping is by working directory: a session is recorded under a prefix
//! only when it ran inside that workspace, so importing never pulls in
//! work done on an unrelated project.

use crate::twin::{latest, observe_src, relate};
use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::{replay, Index, MemIndex};
use brain_store::{now_ms, Store, StoreError};
use std::collections::{BTreeMap, BTreeSet};
use std::io::BufRead;
use std::path::{Path, PathBuf};

/// The longest objective the graph will keep. Enough to recognise a
/// session, far too little to reconstruct a conversation.
const OBJECTIVE_MAX: usize = 200;

/// Bumped whenever this module reads a transcript differently.
///
/// Idempotence is keyed on transcript length, so without this a parser
/// fix would never reach sessions that had already been imported — the
/// same reason `brain twin refresh --full` exists.
const PARSER_VERSION: u32 = 2;

/// What a transcript tells us about one session.
#[derive(Debug, Clone, Default)]
pub struct SessionFacts {
    pub id: String,
    pub agent: String,
    pub cwd: String,
    pub model: Option<String>,
    pub branch: Option<String>,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    /// User instructions in the session — the number of times a person
    /// steered it.
    pub turns: usize,
    pub tool_calls: usize,
    pub tools: BTreeMap<String, usize>,
    /// Absolute paths the session edited or wrote.
    pub touched: BTreeSet<String>,
    pub objective: String,
    /// Transcript length in lines: the idempotence key. A session that
    /// has not grown is not re-read.
    pub lines: u64,
}

impl SessionFacts {
    /// The tool mix, most-used first, as one readable line.
    pub fn tool_summary(&self) -> String {
        let mut rows: Vec<(&String, &usize)> = self.tools.iter().collect();
        rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        rows.iter()
            .take(8)
            .map(|(name, count)| format!("{name} {count}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Transcripts on this machine, newest file first.
///
/// `home` is the user's home directory; both agents keep their history
/// there. Missing directories are simply no sessions.
pub fn transcripts(home: &Path, agent: Option<&str>) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    if agent.is_none_or(|a| a == "claude") {
        // ~/.claude/projects/<slug>/<session-uuid>.jsonl
        for project in read_dir_sorted(&home.join(".claude/projects")) {
            for file in read_dir_sorted(&project) {
                if file.extension().is_some_and(|e| e == "jsonl") {
                    out.push(("claude".to_string(), file));
                }
            }
        }
    }
    if agent.is_none_or(|a| a == "codex") {
        // ~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl
        let mut stack = vec![home.join(".codex/sessions")];
        while let Some(dir) = stack.pop() {
            for entry in read_dir_sorted(&dir) {
                if entry.is_dir() {
                    stack.push(entry);
                } else if entry
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"))
                {
                    out.push(("codex".to_string(), entry));
                }
            }
        }
    }
    out
}

fn read_dir_sorted(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// The working directory a transcript ran in, read from its opening
/// records only.
///
/// Both agents state the cwd in their first few lines, and most
/// transcripts on a machine belong to other projects. Reading the header
/// before committing to a full parse is the difference between scanning a
/// few kilobytes and scanning every megabyte of every session ever run.
pub fn peek_cwd(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    for line in std::io::BufReader::new(file).lines().take(40) {
        let Ok(line) = line else { break };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let cwd = value
            .get("cwd")
            .or_else(|| value.pointer("/payload/cwd"))
            .and_then(|v| v.as_str());
        if let Some(cwd) = cwd.filter(|c| !c.is_empty()) {
            return Some(cwd.to_string());
        }
    }
    None
}

/// Read a transcript. Returns `None` when the file holds no session.
pub fn parse(agent: &str, path: &Path) -> Option<SessionFacts> {
    let file = std::fs::File::open(path).ok()?;
    let reader = std::io::BufReader::new(file);
    let mut facts = SessionFacts {
        agent: agent.to_string(),
        ..Default::default()
    };
    // A transcript line can be megabytes of tool output; read line by
    // line and keep only the handful of fields that matter.
    for line in reader.lines() {
        let Ok(line) = line else { break };
        facts.lines += 1;
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        match agent {
            "claude" => absorb_claude(&mut facts, &value),
            "codex" => absorb_codex(&mut facts, &value),
            _ => {}
        }
    }
    if facts.id.is_empty() {
        // Fall back to the filename stem, which both agents derive from
        // the session id.
        facts.id = path.file_stem()?.to_string_lossy().to_string();
    }
    (facts.started_at_ms > 0).then_some(facts)
}

fn absorb_claude(facts: &mut SessionFacts, value: &serde_json::Value) {
    // Sub-agent transcripts are interleaved; they are not the session.
    if value.get("isSidechain").and_then(|v| v.as_bool()) == Some(true) {
        return;
    }
    if let Some(id) = value.get("sessionId").and_then(|v| v.as_str()) {
        if facts.id.is_empty() {
            facts.id = id.to_string();
        }
    }
    if let Some(cwd) = value.get("cwd").and_then(|v| v.as_str()) {
        if facts.cwd.is_empty() {
            facts.cwd = cwd.to_string();
        }
    }
    if let Some(branch) = value
        .get("gitBranch")
        .and_then(|v| v.as_str())
        .filter(|b| !b.is_empty())
    {
        facts.branch = Some(branch.to_string());
    }
    if let Some(at) = value
        .get("timestamp")
        .and_then(|v| v.as_str())
        .and_then(parse_iso8601_ms)
    {
        note_time(facts, at);
    }

    let Some(message) = value.get("message") else {
        return;
    };
    if let Some(model) = message.get("model").and_then(|v| v.as_str()) {
        facts.model = Some(model.to_string());
    }
    let kind = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let content = message.get("content");

    if kind == "user" {
        // A tool's output comes back as a `user` record too. Only a
        // record carrying human text is a turn — otherwise a session with
        // thirty instructions reports seven hundred.
        let text = match content {
            Some(serde_json::Value::String(text)) => Some(text.clone()),
            Some(serde_json::Value::Array(blocks)) => {
                if blocks
                    .iter()
                    .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result"))
                {
                    None
                } else {
                    blocks
                        .iter()
                        .find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                        .and_then(|b| b.get("text").and_then(|t| t.as_str()))
                        .map(str::to_string)
                }
            }
            _ => None,
        };
        if let Some(text) = text.filter(|t| !is_injected(t)) {
            facts.turns += 1;
            if facts.objective.is_empty() {
                facts.objective = objective(&text);
            }
        }
    }

    if let Some(blocks) = content.and_then(|c| c.as_array()) {
        for block in blocks {
            if block.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                continue;
            }
            let name = block
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("tool")
                .to_string();
            facts.tool_calls += 1;
            *facts.tools.entry(name.clone()).or_insert(0) += 1;
            // Only the tools that change files contribute paths; a Read
            // is not a touch, and recording every read would drown the
            // signal in noise.
            if !matches!(name.as_str(), "Edit" | "Write" | "NotebookEdit") {
                continue;
            }
            let Some(input) = block.get("input") else {
                continue;
            };
            for key in ["file_path", "notebook_path"] {
                if let Some(path) = input.get(key).and_then(|p| p.as_str()) {
                    facts.touched.insert(path.to_string());
                }
            }
        }
    }
}

fn absorb_codex(facts: &mut SessionFacts, value: &serde_json::Value) {
    if let Some(at) = value
        .get("timestamp")
        .and_then(|v| v.as_str())
        .and_then(parse_iso8601_ms)
    {
        note_time(facts, at);
    }
    let Some(payload) = value.get("payload") else {
        return;
    };
    match value.get("type").and_then(|v| v.as_str()).unwrap_or("") {
        "session_meta" => {
            if let Some(id) = payload.get("id").and_then(|v| v.as_str()) {
                facts.id = id.to_string();
            }
            if let Some(cwd) = payload.get("cwd").and_then(|v| v.as_str()) {
                facts.cwd = cwd.to_string();
            }
        }
        "turn_context" => {
            if let Some(model) = payload.get("model").and_then(|v| v.as_str()) {
                facts.model = Some(model.to_string());
            }
            if facts.cwd.is_empty() {
                if let Some(cwd) = payload.get("cwd").and_then(|v| v.as_str()) {
                    facts.cwd = cwd.to_string();
                }
            }
        }
        "response_item" => absorb_codex_item(facts, payload),
        _ => {}
    }
}

fn absorb_codex_item(facts: &mut SessionFacts, payload: &serde_json::Value) {
    match payload.get("type").and_then(|v| v.as_str()).unwrap_or("") {
        "message" if payload.get("role").and_then(|r| r.as_str()) == Some("user") => {
            let text = payload
                .get("content")
                .and_then(|c| c.as_array())
                .and_then(|blocks| {
                    blocks
                        .iter()
                        .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                        .find(|t| !is_injected(t))
                })
                .map(str::to_string);
            if let Some(text) = text {
                facts.turns += 1;
                if facts.objective.is_empty() {
                    facts.objective = objective(&text);
                }
            }
        }
        "function_call" | "custom_tool_call" => {
            let name = payload
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("tool")
                .to_string();
            facts.tool_calls += 1;
            *facts.tools.entry(name.clone()).or_insert(0) += 1;
            if name == "apply_patch" {
                if let Some(patch) = payload.get("input").and_then(|i| i.as_str()) {
                    for path in patched_files(patch) {
                        facts.touched.insert(path);
                    }
                }
            }
        }
        _ => {}
    }
}

/// Files named by an `apply_patch` envelope.
fn patched_files(patch: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in patch.lines() {
        for marker in ["*** Update File: ", "*** Add File: ", "*** Delete File: "] {
            if let Some(rest) = line.strip_prefix(marker) {
                out.push(rest.trim().to_string());
            }
        }
    }
    out
}

fn note_time(facts: &mut SessionFacts, at_ms: u64) {
    if facts.started_at_ms == 0 || at_ms < facts.started_at_ms {
        facts.started_at_ms = at_ms;
    }
    facts.ended_at_ms = facts.ended_at_ms.max(at_ms);
}

/// Text the harness put in the conversation, not the person.
///
/// Both agents inject context as ordinary user turns: XML-ish context
/// blocks, the repository's own agent instructions, role preambles. None
/// of it is an objective, and counting it inflates the turn count.
fn is_injected(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with('<')
        || text.starts_with("You are ")
        || (text.starts_with('#') && text.contains("instructions for"))
        || text.starts_with("Caveat:")
        || text.starts_with("This session is being continued")
}

/// The first instruction, trimmed to something recognisable.
fn objective(text: &str) -> String {
    let text = text.trim();
    // Command invocations and pasted system blocks are not objectives.
    let text = text
        .lines()
        .find(|l| {
            let l = l.trim();
            !l.is_empty() && !l.starts_with('<')
        })
        .unwrap_or("")
        .trim();
    if text.chars().count() <= OBJECTIVE_MAX {
        return text.to_string();
    }
    let cut = text
        .char_indices()
        .nth(OBJECTIVE_MAX)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    format!("{}…", text[..cut].trim_end())
}

/// `2026-07-27T06:11:29.683Z` → epoch milliseconds.
///
/// Both agents write RFC 3339 in UTC; a four-line conversion beats a
/// date-time dependency for a format that is fixed by the writer.
fn parse_iso8601_ms(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let num = |from: usize, to: usize| text.get(from..to)?.parse::<i64>().ok();
    let (year, month, day) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hour, min, sec) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    let millis = text
        .get(20..23)
        .filter(|_| bytes.get(19) == Some(&b'.'))
        .and_then(|m| m.parse::<i64>().ok())
        .unwrap_or(0);

    // Days since the Unix epoch, by the civil-from-days algorithm.
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;

    let secs = days * 86_400 + hour * 3_600 + min * 60 + sec;
    u64::try_from(secs * 1_000 + millis).ok()
}

// ---------------------------------------------------------------------------
// Recording
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct ImportOutcome {
    pub imported: usize,
    pub unchanged: usize,
    /// Sessions skipped because they ran somewhere else.
    pub elsewhere: usize,
}

/// Import every transcript that ran inside `root` into the graph.
pub fn import(
    store: &Store,
    home: &Path,
    root: &Path,
    prefix: &str,
    agent: Option<&str>,
    since_ms: u64,
) -> Result<ImportOutcome, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    let now = now_ms();
    let workspace = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let files = crate::twin::twinned_paths(store, prefix)?;
    let mut out = ImportOutcome::default();
    let mut written = BTreeSet::new();

    for (agent, path) in transcripts(home, agent) {
        // A session belongs to this workspace only if it ran there, and
        // that is knowable from the header alone.
        let Some(cwd) = peek_cwd(&path) else {
            continue;
        };
        let resolved = PathBuf::from(&cwd);
        let resolved = resolved.canonicalize().unwrap_or(resolved);
        if !resolved.starts_with(&workspace) {
            out.elsewhere += 1;
            continue;
        }
        let Some(facts) = parse(&agent, &path) else {
            continue;
        };
        if facts.ended_at_ms < since_ms {
            continue;
        }
        if record(store, &index, &mut written, prefix, &facts, &files, &workspace, now)? {
            out.imported += 1;
        } else {
            out.unchanged += 1;
        }
    }
    Ok(out)
}

/// Record one session's parsed facts — the unit `import` applies per
/// transcript, public so tests and other ingest paths can record a
/// session without a transcript file on disk.
pub fn record_facts(
    store: &Store,
    prefix: &str,
    facts: &SessionFacts,
    workspace: &Path,
) -> Result<bool, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    let files = crate::twin::twinned_paths(store, prefix)?;
    let mut written = BTreeSet::new();
    let workspace = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    record(
        store, &index, &mut written, prefix, facts, &files, &workspace, now_ms(),
    )
}

pub fn session_sid(prefix: &str, id: &str) -> StableId {
    StableId::derive(&["session", prefix, id])
}

/// The outcomes a session can carry once the fate of its work is known.
/// A note from an abandoned approach deserves different weight than one
/// that shipped — this is where that weight comes from.
pub const OUTCOMES: &[&str] = &["shipped", "abandoned", "superseded"];

/// Annotate a session after the fact: the distilled objective (what it
/// was really about) and the outcome (whether its work survived). New
/// observations supersede the import-time guess; the guess stays in the
/// timeline.
pub fn annotate(
    store: &Store,
    prefix: &str,
    id: &str,
    objective: Option<&str>,
    outcome: Option<&str>,
) -> Result<StableId, StoreError> {
    let sid = session_sid(prefix, id);
    let now = now_ms();
    if let Some(o) = objective {
        observe_src(store, &sid, "objective", o, "agent", now)?;
    }
    if let Some(o) = outcome {
        observe_src(store, &sid, "outcome", o, "agent", now)?;
    }
    Ok(sid)
}

/// Write one session. Returns false when the transcript has not grown
/// since the last import, in which case nothing is read or written.
#[allow(clippy::too_many_arguments)]
fn record(
    store: &Store,
    index: &MemIndex,
    written: &mut BTreeSet<(StableId, String, StableId)>,
    prefix: &str,
    facts: &SessionFacts,
    twinned: &BTreeSet<String>,
    workspace: &Path,
    now: u64,
) -> Result<bool, StoreError> {
    let sid = session_sid(prefix, &facts.id);
    let lines = format!("{}@v{PARSER_VERSION}", facts.lines);
    if latest(index, store, &sid, "transcript_lines")?.as_deref() == Some(lines.as_str()) {
        return Ok(false);
    }

    let mut labels = BTreeMap::new();
    labels.insert("prefix".to_string(), prefix.to_string());
    labels.insert("session_id".to_string(), facts.id.clone());
    labels.insert("agent".to_string(), facts.agent.clone());
    labels.insert("cwd".to_string(), facts.cwd.clone());
    store.put(&Object::Entity {
        id: sid.clone(),
        entity_kind: "agent_session".to_string(),
        labels,
    })?;

    let mut props: Vec<(&str, String)> = vec![
        ("objective", facts.objective.clone()),
        ("started_at", facts.started_at_ms.to_string()),
        ("ended_at", facts.ended_at_ms.to_string()),
        ("turns", facts.turns.to_string()),
        ("tool_calls", facts.tool_calls.to_string()),
        ("tools", facts.tool_summary()),
        ("files_touched", facts.touched.len().to_string()),
        ("transcript_lines", lines),
    ];
    if let Some(model) = &facts.model {
        props.push(("model", model.clone()));
    }
    if let Some(branch) = &facts.branch {
        props.push(("branch", branch.clone()));
    }
    for (property, value) in props {
        if value.is_empty() {
            continue;
        }
        if latest(index, store, &sid, property)?.as_deref() != Some(value.as_str()) {
            observe_src(store, &sid, property, &value, "sessions", now)?;
        }
    }

    let repo_sid = StableId::derive(&["repo", prefix]);
    relate(store, index, written, &sid, "concerns", &repo_sid, now)?;
    for absolute in &facts.touched {
        let Some(rel) = relative_to(workspace, absolute) else {
            continue; // edited outside the workspace; not this graph's business
        };
        if !twinned.contains(&rel) {
            continue; // not a file the twin tracks
        }
        let file_sid = StableId::derive(&["file", &rel]);
        relate(store, index, written, &sid, "touched", &file_sid, now)?;
    }
    Ok(true)
}

fn relative_to(root: &Path, absolute: &str) -> Option<String> {
    let path = PathBuf::from(absolute);
    let path = path.canonicalize().unwrap_or(path);
    let rel = path.strip_prefix(root).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionRow {
    pub sid: StableId,
    pub id: String,
    pub agent: String,
    pub objective: String,
    /// shipped | abandoned | superseded, once someone judged it.
    pub outcome: Option<String>,
    pub model: Option<String>,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub turns: usize,
    pub tools: String,
    pub files_touched: usize,
}

/// Recorded sessions under a prefix, most recent first.
pub fn list(store: &Store, index: &MemIndex, prefix: &str) -> Result<Vec<SessionRow>, StoreError> {
    let mut seen: BTreeSet<StableId> = BTreeSet::new();
    let mut out = Vec::new();
    for node in index.entities_by_kind("agent_session") {
        let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
            continue;
        };
        if labels.get("prefix").map(String::as_str) != Some(prefix) || !seen.insert(id.clone()) {
            continue;
        }
        let number = |property: &str| -> u64 {
            latest(index, store, &id, property)
                .ok()
                .flatten()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0)
        };
        out.push(SessionRow {
            id: labels.get("session_id").cloned().unwrap_or_default(),
            agent: labels.get("agent").cloned().unwrap_or_default(),
            objective: latest(index, store, &id, "objective")?.unwrap_or_default(),
            outcome: latest(index, store, &id, "outcome")?,
            model: latest(index, store, &id, "model")?,
            started_at_ms: number("started_at"),
            ended_at_ms: number("ended_at"),
            turns: number("turns") as usize,
            tools: latest(index, store, &id, "tools")?.unwrap_or_default(),
            files_touched: number("files_touched") as usize,
            sid: id,
        });
    }
    out.sort_by(|a, b| b.ended_at_ms.cmp(&a.ended_at_ms));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_index(store: &Store) -> MemIndex {
        let mut index = MemIndex::new();
        replay(store, &mut index).unwrap();
        index
    }

    #[test]
    fn annotate_supersedes_objective_and_records_outcome() {
        let work = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let root = work.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/login.rs"), "pub fn login() {}\n").unwrap();
        claude_transcript(home.path(), root.to_str().unwrap());
        let store = Store::open(root.join(".brain")).unwrap();
        crate::twin::refresh(&store, &root, "twin/app").unwrap();
        import(&store, home.path(), &root, "twin/app", None, 0).unwrap();

        annotate(
            &store,
            "twin/app",
            "sess-1",
            Some("Debounce the login redraw loop"),
            Some("shipped"),
        )
        .unwrap();

        let index = fresh_index(&store);
        let rows = list(&store, &index, "twin/app").unwrap();
        assert_eq!(rows[0].objective, "Debounce the login redraw loop");
        assert_eq!(rows[0].outcome.as_deref(), Some("shipped"));
        // The import-time guess is history, not gone: the observation
        // timeline keeps both.
    }

    fn claude_transcript(home: &Path, cwd: &str) -> PathBuf {
        let dir = home.join(".claude/projects/-Users-x-app");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sess-1.jsonl");
        let lines = [
            format!(
                r#"{{"type":"user","sessionId":"sess-1","cwd":"{cwd}","gitBranch":"main","timestamp":"2026-07-27T06:11:29.683Z","message":{{"role":"user","content":[{{"type":"text","text":"Make the login page stop flickering.\nIt is very annoying."}}]}}}}"#
            ),
            format!(
                r#"{{"type":"assistant","sessionId":"sess-1","cwd":"{cwd}","timestamp":"2026-07-27T06:12:00.000Z","message":{{"role":"assistant","model":"claude-fable-5","content":[{{"type":"tool_use","name":"Read","input":{{"file_path":"{cwd}/src/login.rs"}}}},{{"type":"tool_use","name":"Edit","input":{{"file_path":"{cwd}/src/login.rs"}}}}]}}}}"#
            ),
            // A tool's output arrives as a `user` record; it is not a turn.
            format!(
                r#"{{"type":"user","sessionId":"sess-1","cwd":"{cwd}","timestamp":"2026-07-27T06:12:05.000Z","message":{{"role":"user","content":[{{"type":"tool_result","content":"ok"}}]}}}}"#
            ),
            // Neither is a context block the harness injected.
            format!(
                r#"{{"type":"user","sessionId":"sess-1","cwd":"{cwd}","timestamp":"2026-07-27T06:12:06.000Z","message":{{"role":"user","content":[{{"type":"text","text":"<system-reminder>be good</system-reminder>"}}]}}}}"#
            ),
            // A sub-agent's work is not the session's own record.
            format!(
                r#"{{"type":"assistant","isSidechain":true,"sessionId":"sess-1","cwd":"{cwd}","timestamp":"2026-07-27T06:13:00.000Z","message":{{"role":"assistant","content":[{{"type":"tool_use","name":"Edit","input":{{"file_path":"{cwd}/src/elsewhere.rs"}}}}]}}}}"#
            ),
        ];
        std::fs::write(&path, lines.join("\n")).unwrap();
        path
    }

    #[test]
    fn a_claude_session_becomes_an_actor_with_an_objective_and_a_blast_radius() {
        let work = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let root = work.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/login.rs"), "pub fn login() {}\n").unwrap();
        claude_transcript(home.path(), root.to_str().unwrap());

        let store = Store::open(&root.join(".brain")).unwrap();
        crate::twin::refresh(&store, &root, "twin/app").unwrap();
        let out = import(&store, home.path(), &root, "twin/app", None, 0).unwrap();
        assert_eq!((out.imported, out.unchanged), (1, 0));

        let index = fresh_index(&store);
        let rows = list(&store, &index, "twin/app").unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.agent, "claude");
        assert_eq!(row.objective, "Make the login page stop flickering.");
        assert_eq!(row.model.as_deref(), Some("claude-fable-5"));
        assert_eq!(
            row.turns, 1,
            "one human instruction; tool results and injected context are not turns"
        );
        assert_eq!(row.tools, "Edit 1, Read 1");
        assert_eq!(row.files_touched, 1, "a Read is not a touch");
        assert!(row.started_at_ms > 0 && row.ended_at_ms >= row.started_at_ms);

        // The session is joined to the files it changed — the blast radius
        // of an agent's work becomes a graph question.
        let touched = crate::twin::live_from(&index, &store, &row.sid, "touched").unwrap();
        assert_eq!(touched.len(), 1);
        assert_eq!(touched[0].1, StableId::derive(&["file", "src/login.rs"]));

        // Re-importing an unchanged transcript writes nothing.
        let before = store.count_objects().unwrap();
        let again = import(&store, home.path(), &root, "twin/app", None, 0).unwrap();
        assert_eq!((again.imported, again.unchanged), (0, 1));
        assert_eq!(store.count_objects().unwrap(), before);
    }

    #[test]
    fn a_transcript_never_puts_the_conversation_into_the_graph() {
        let work = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let root = work.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/login.rs"), "pub fn login() {}\n").unwrap();
        claude_transcript(home.path(), root.to_str().unwrap());

        let store = Store::open(&root.join(".brain")).unwrap();
        crate::twin::refresh(&store, &root, "twin/app").unwrap();
        import(&store, home.path(), &root, "twin/app", None, 0).unwrap();

        // Everything the store holds, as text.
        let mut everything = String::new();
        for node in store.put_history().unwrap() {
            if let Ok(object) = store.get(&node) {
                everything.push_str(&format!("{object:?}\n"));
            }
        }
        assert!(
            everything.contains("Make the login page stop flickering."),
            "the objective is kept — a session must be identifiable"
        );
        assert!(
            !everything.contains("It is very annoying"),
            "the rest of the instruction is not the graph's business"
        );
        assert!(
            !everything.contains("elsewhere.rs"),
            "a sub-agent's edits are not the session's record"
        );
    }

    #[test]
    fn sessions_from_other_workspaces_are_left_alone() {
        let work = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let root = work.path().canonicalize().unwrap();
        std::fs::write(root.join("a.rs"), "pub fn a() {}\n").unwrap();
        claude_transcript(home.path(), "/somewhere/else");

        let store = Store::open(&root.join(".brain")).unwrap();
        crate::twin::refresh(&store, &root, "twin/app").unwrap();
        let out = import(&store, home.path(), &root, "twin/app", None, 0).unwrap();
        assert_eq!((out.imported, out.elsewhere), (0, 1));
        assert!(list(&store, &fresh_index(&store), "twin/app")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn a_codex_rollout_reads_its_objective_model_and_patched_files() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".codex/sessions/2026/06/09");
        std::fs::create_dir_all(&dir).unwrap();
        let lines = [
            r#"{"type":"session_meta","timestamp":"2026-06-09T14:46:28.456Z","payload":{"id":"019eacd9","cwd":"/work","originator":"codex-tui"}}"#,
            r#"{"type":"turn_context","timestamp":"2026-06-09T14:46:31.379Z","payload":{"model":"gpt-5.5","cwd":"/work"}}"#,
            r#"{"type":"response_item","timestamp":"2026-06-09T14:46:31.500Z","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"<permissions instructions>ignore me"}]}}"#,
            r#"{"type":"response_item","timestamp":"2026-06-09T14:46:32.000Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Add a health endpoint."}]}}"#,
            r#"{"type":"response_item","timestamp":"2026-06-09T14:47:00.000Z","payload":{"type":"function_call","name":"exec_command","arguments":"{}"}}"#,
            r#"{"type":"response_item","timestamp":"2026-06-09T14:48:00.000Z","payload":{"type":"custom_tool_call","name":"apply_patch","input":"*** Begin Patch\n*** Update File: src/health.rs\n*** Add File: src/new.rs\n*** End Patch"}}"#,
        ];
        std::fs::write(dir.join("rollout-2026-06-09T16-46-28-019eacd9.jsonl"), lines.join("\n"))
            .unwrap();

        let found = transcripts(home.path(), Some("codex"));
        assert_eq!(found.len(), 1);
        let facts = parse("codex", &found[0].1).unwrap();
        assert_eq!(facts.id, "019eacd9");
        assert_eq!(facts.cwd, "/work");
        assert_eq!(facts.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(
            facts.objective, "Add a health endpoint.",
            "an injected instruction block is not the objective"
        );
        assert_eq!(facts.turns, 1);
        assert_eq!(facts.tool_summary(), "apply_patch 1, exec_command 1");
        assert_eq!(
            facts.touched,
            BTreeSet::from(["src/health.rs".to_string(), "src/new.rs".to_string()])
        );
    }

    #[test]
    fn timestamps_convert_without_a_date_library() {
        assert_eq!(parse_iso8601_ms("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(parse_iso8601_ms("2026-07-27T06:11:29.683Z"), Some(1785132689683));
        // A leap day, and a value with no fractional part.
        assert_eq!(parse_iso8601_ms("2024-02-29T12:00:00Z"), Some(1709208000000));
        assert_eq!(parse_iso8601_ms("nonsense"), None);
    }
}
