//! The voice: one place that turns graph facts into sentences a person can
//! read out loud.
//!
//! Everything the browser displays as prose comes from here, so the wording
//! cannot drift between screens and the frontend never invents a status
//! model (ADR-023). Two rules:
//!
//! 1. **Every number carries its unit and its consequence.** Not "hub 29"
//!    but "29 files import this — edits here ripple widest".
//! 2. **No machine vocabulary in the primary flow.** Content hashes, event
//!    cursors, stable ids and relation predicates are available under a
//!    details disclosure, never in a headline.

/// "3 minutes ago", "yesterday", "2 weeks ago".
pub fn ago(now_ms: u64, then_ms: u64) -> String {
    if then_ms == 0 {
        return "at an unrecorded time".to_string();
    }
    let seconds = now_ms.saturating_sub(then_ms) / 1000;
    match seconds {
        0..=44 => "just now".to_string(),
        45..=5399 => {
            let minutes = (seconds as f64 / 60.0).round().max(1.0) as u64;
            format!("{} ago", count(minutes, "minute", "minutes"))
        }
        5400..=79199 => {
            let hours = (seconds as f64 / 3600.0).round().max(1.0) as u64;
            format!("{} ago", count(hours, "hour", "hours"))
        }
        79200..=129599 => "yesterday".to_string(),
        129600..=1209599 => {
            let days = seconds / 86400;
            format!("{} ago", count(days, "day", "days"))
        }
        _ => {
            let weeks = seconds / 604800;
            format!("{} ago", count(weeks, "week", "weeks"))
        }
    }
}

/// "1 file" / "3 files".
pub fn count(n: u64, singular: &str, plural: &str) -> String {
    if n == 1 {
        format!("1 {singular}")
    } else {
        format!("{n} {plural}")
    }
}

/// The human noun for an entity kind. The graph's kind strings are
/// implementation vocabulary; these are what a person would say.
pub fn kind_noun(kind: &str) -> &'static str {
    match kind {
        "source_file" => "file",
        "symbol" => "function or type",
        "module" => "external dependency",
        "repo" => "repository",
        "decision" => "decision",
        "plan" => "plan",
        "doc" => "document",
        "runbook" => "runbook",
        "skill" => "agent skill",
        "agent_config" => "agent instructions",
        "template" => "contract",
        "feature" => "feature",
        "test_run" => "test run",
        "test_case" => "test",
        "change" => "governed change",
        "asset" => "asset",
        "prototype" => "prototype",
        "task_list" => "task list",
        "capability_matrix" => "capability matrix",
        "run_log" => "run log",
        "agent_session" => "agent session",
        other => leak_noun(other),
    }
}

/// Kinds taught at runtime have no compiled noun; fall back to their own
/// name with underscores softened.
fn leak_noun(kind: &str) -> &'static str {
    match kind {
        "" => "entity",
        _ => "record",
    }
}

/// A shape token for the UI. Shape carries kind so colour never has to
/// carry it alone — taken from the design draft's glyph vocabulary.
pub fn kind_glyph(kind: &str) -> &'static str {
    match kind {
        "feature" => "hexagon",
        "test_case" | "test_run" => "diamond",
        "decision" => "kite",
        "doc" | "runbook" | "plan" | "task_list" | "skill" | "agent_config" => "page",
        "asset" | "prototype" | "capability_matrix" => "square",
        "change" => "chevron",
        "template" => "shield",
        "module" => "circle",
        "agent_session" => "orbit",
        _ => "block",
    }
}

/// A test's recorded verdict.
pub fn test_result(result: &str) -> (&'static str, &'static str) {
    match result {
        "pass" => ("passing", "good"),
        "fail" => ("failing", "bad"),
        "skip" => ("skipped", "quiet"),
        _ => ("no result recorded", "quiet"),
    }
}

/// The framework a test belongs to, as its makers spell it.
pub fn framework_noun(framework: &str) -> &'static str {
    match framework {
        "rust" => "Rust",
        "playwright" => "Playwright",
        "jest" => "Jest",
        "pytest" => "pytest",
        "phpunit" => "PHPUnit",
        _ => "tests",
    }
}

/// What a run report format is called in conversation.
pub fn report_format(format: &str) -> &'static str {
    match format {
        "cargo" => "cargo test",
        "junit" => "a JUnit report",
        "playwright" => "Playwright",
        _ => "an imported report",
    }
}

/// A duration in milliseconds, rounded to something worth saying.
pub fn duration(ms: u64) -> String {
    match ms {
        0..=999 => format!("{ms} ms"),
        1000..=59_999 => {
            let seconds = (ms as f64 / 100.0).round() / 10.0;
            format!("{seconds:.1} seconds")
        }
        60_000..=3_599_999 => count((ms as f64 / 60_000.0).round() as u64, "minute", "minutes"),
        _ => count((ms as f64 / 3_600_000.0).round() as u64, "hour", "hours"),
    }
}

/// What an attached file is, by its subtype.
pub fn attachment_noun(subtype: &str) -> &'static str {
    match subtype {
        "image" => "screenshot",
        "screencast" => "recording",
        "audio" => "audio",
        "trace" => "trace",
        "template" => "template",
        _ => "file",
    }
}

/// What a level of verification actually establishes. The taxonomy is the
/// substrate's; these are the claims it licenses.
pub fn evidence_level(level: &str) -> &'static str {
    match level {
        "behavioral" => "it was run and observed",
        "structural" => "its shape was checked",
        "empirical" => "it was measured",
        "formal" => "it was proved",
        "interpretive" => "someone judged it",
        "transactional" => "the effect was confirmed",
        "authorization" => "someone authorised it",
        _ => "nothing supports it yet",
    }
}

/// Marks a value Eyes worked out rather than read.
///
/// The graph records no command line, no actor for historical intents,
/// and no owner for an inferred link. Where Eyes reconstructs one, it
/// must say so — a plausible sentence presented as a record is the exact
/// failure this whole system exists to prevent.
pub const RECONSTRUCTED: &str = "reconstructed from what the graph records, not itself recorded";

/// How long a session ran.
pub fn span(from_ms: u64, to_ms: u64) -> String {
    if from_ms == 0 || to_ms <= from_ms {
        return "a moment".to_string();
    }
    duration(to_ms - from_ms)
}

/// Which agent did the work.
pub fn agent_noun(agent: &str) -> &'static str {
    match agent {
        "claude" => "Claude Code",
        "codex" => "Codex",
        _ => "an agent",
    }
}

/// Turn one raw attention reason into a sentence, or drop it when it says
/// nothing a person can act on. The raw strings come from
/// `brain_observe::attention`, which is our own code and stable.
pub fn attention_reason(raw: &str) -> Option<String> {
    if let Some(rest) = raw.strip_prefix("hub ") {
        let n: u64 = rest.trim().parse().ok()?;
        let subject = if n == 1 {
            "1 file imports this".to_string()
        } else {
            format!("{n} files import this")
        };
        return Some(format!("{subject} — edits here ripple widest"));
    }
    if raw == "untested hub" {
        return Some("no test covers it".to_string());
    }
    if let Some(rest) = raw.strip_prefix("churn ") {
        // "churn 4 (1 recent)"
        let (lifetime, recent) = parse_churn(rest)?;
        if recent == 1 {
            return Some("changed once since your last session".to_string());
        }
        if recent > 1 {
            return Some(format!("changed {recent} times since your last session"));
        }
        if lifetime >= 5 {
            return Some(format!(
                "changed {} over its history",
                count(lifetime, "time", "times")
            ));
        }
        return None;
    }
    if let Some(rest) = raw.strip_suffix(" failing test(s)") {
        let n: u64 = rest.trim().parse().ok()?;
        return Some(format!("{} failing", count(n, "test is", "tests are")));
    }
    if let Some(rest) = raw.strip_prefix("stale: ") {
        return Some(format!(
            "the code changed after this was written ({})",
            rest.trim_end_matches(" changed since")
        ));
    }
    if let Some(rest) = raw.strip_prefix("stale (info): ") {
        return Some(format!(
            "written before later changes to {}",
            rest.trim_end_matches(" changed since")
        ));
    }
    if let Some(rest) = raw.strip_prefix("nonconforming: missing ") {
        return Some(format!("missing {rest}"));
    }
    if raw.starts_with("status '") {
        return Some("claims to be shipped while a requirement is unmet".to_string());
    }
    Some(raw.to_string())
}

fn parse_churn(rest: &str) -> Option<(u64, u64)> {
    let (lifetime, tail) = rest.split_once(" (")?;
    let recent = tail.trim_end_matches(" recent)");
    Some((lifetime.trim().parse().ok()?, recent.trim().parse().ok()?))
}

/// How a document's freshness reads to a person.
pub fn freshness(severity: Option<&str>, lifecycle_active: bool) -> (&'static str, &'static str) {
    match (severity, lifecycle_active) {
        (_, false) => ("historical", "kept as a record — not expected to match today's code"),
        (Some("warn"), _) => ("may be wrong", "the code changed after this was written"),
        (Some("info"), _) => ("aging", "written before later changes — normal for a record"),
        _ => ("current", "matches the code as last observed"),
    }
}

/// The sentence for a lifecycle state, or `None` when the thing is simply
/// current and needs no explanation.
pub fn lifecycle(state: &str, why: &str) -> Option<String> {
    let base = match state {
        "active" => return None,
        "done" => "finished",
        "abandoned" => "abandoned",
        "retired" => "retired",
        "superseded" => "superseded",
        _ => return None,
    };
    if why.is_empty() {
        Some(base.to_string())
    } else {
        Some(format!("{base} — {}", humanize_why(why)))
    }
}

/// Lifecycle reasons are written for the CLI; soften the few that leak
/// vocabulary.
fn humanize_why(why: &str) -> String {
    if let Some(rest) = why.strip_prefix("superseded by ") {
        return format!("replaced by {rest}");
    }
    if why == "source file deleted" {
        return "the file it lived in was deleted".to_string();
    }
    if let Some(rest) = why.strip_prefix("status '") {
        return format!("marked {}", rest.trim_end_matches('\''));
    }
    if let Some(rest) = why.strip_prefix("set by agent: ") {
        return rest.to_string();
    }
    if why == "set by agent" {
        return "set explicitly".to_string();
    }
    why.to_string()
}

/// A coherence finding, as a sentence plus the command that resolves it.
pub fn finding(kind: &str, label: &str, detail: &str) -> (String, String) {
    let sentence = match kind {
        k if k.starts_with("dangling-mention") => {
            format!("{label} points at a file that no longer exists")
        }
        "dangling-test" => format!("the test {label} lives in a file that was deleted"),
        "stuck-change" => format!("the change {label} never finished"),
        "broken-change" => format!("the change {label} was applied but its tests failed"),
        "incoherent-feature" => format!("{label} says it is shipped, but it is not"),
        "orphaned-asset" => format!("{label} belongs to something that is finished"),
        k if k.starts_with("active-but-homeless") => {
            format!("{label} is marked active, but its files are gone")
        }
        _ => format!("{label}: {detail}"),
    };
    (sentence, detail.to_string())
}

/// The plural of a kind's noun, for a group heading.
pub fn kind_plural(kind: &str) -> String {
    let noun = kind_noun(kind);
    match noun {
        "function or type" => "functions and types".to_string(),
        "agent instructions" => noun.to_string(),
        other if other.ends_with('s') => other.to_string(),
        other => format!("{other}s"),
    }
}

/// What a feature reaches, in one sentence.
///
/// The two halves are different kinds of statement and are kept apart: a
/// feature *declares* its evidence, and *reaches* whatever the twin
/// already pointed at those files by itself.
pub fn reach_sentence(declared: usize, reached: usize, files: usize) -> String {
    if files == 0 {
        return "It declares no files, so nothing reaches it.".to_string();
    }
    if reached == 0 {
        return format!(
            "It declares {}. Nothing else in the graph points at {}.",
            count(declared as u64, "record", "records"),
            if files == 1 { "it" } else { "them" }
        );
    }
    format!(
        "It declares {}. Through {}, it reaches {} nobody linked by hand.",
        count(declared as u64, "record", "records"),
        count(files as u64, "file", "files"),
        count(reached as u64, "more record", "more records")
    )
}

/// Why a record is attributed to a feature, read from the record's side.
///
/// A derived attribution always names the file it came through. Saying
/// only *that* something belongs to a feature, without saying how it was
/// reached, would be a claim nobody could check.
pub fn attribution_because(via: &str, predicate: &str, through: Option<&str>) -> String {
    match (via, through) {
        ("declared", _) => format!("it {} this feature", predicate_phrase(predicate, false)),
        ("part", _) => format!(
            "it {} one of its parts",
            predicate_phrase(predicate, false)
        ),
        // The case is defined in the test file, and that file covers the
        // declared one — two hops, and the sentence says both.
        ("suite", Some(file)) => {
            format!("it is defined in a file that tests {file}, which this feature is built by")
        }
        (_, Some(file)) => format!(
            "it {} {file}, which this feature is built by",
            predicate_phrase(predicate, true)
        ),
        (_, None) => format!("it {} this feature", predicate_phrase(predicate, false)),
    }
}

/// Human phrasing for a definition-of-done predicate.
pub fn dod_label(predicate: &str) -> &'static str {
    match predicate {
        "implemented_by" => "built",
        "tested_by" => "tested",
        "decided_by" => "decided",
        "documented_in" => "documented",
        "part_of" => "part of",
        _ => "linked",
    }
}

/// Human phrasing for a relation predicate, used in neighbourhood and
/// relationship lists.
pub fn predicate_phrase(predicate: &str, outgoing: bool) -> String {
    let phrase = match (predicate, outgoing) {
        ("imports", true) => "uses",
        ("imports", false) => "is used by",
        ("contains", true) => "defines",
        ("contains", false) => "is defined in",
        ("covers", true) => "tests",
        ("covers", false) => "is tested by",
        ("mentions", true) => "mentions",
        ("mentions", false) => "is mentioned by",
        ("recorded_in", true) => "is written in",
        ("recorded_in", false) => "holds",
        ("projected_to", true) => "is rendered to",
        ("projected_to", false) => "is rendered from",
        ("supersedes", true) => "replaces",
        ("supersedes", false) => "was replaced by",
        ("conforms_to", true) => "follows the contract",
        ("conforms_to", false) => "is the contract for",
        ("concerns", true) => "belongs to",
        ("concerns", false) => "includes",
        // Read from the other end, a definition-of-done edge says what the
        // record does for the feature. Without these the fall-through
        // produced "is implemented by by" on every implementation file.
        ("implemented_by", true) => "is built by",
        ("implemented_by", false) => "builds",
        ("tested_by", true) => "is tested by",
        ("tested_by", false) => "tests",
        ("decided_by", true) => "was decided by",
        ("decided_by", false) => "decides",
        ("documented_in", true) => "is documented in",
        ("documented_in", false) => "documents",
        ("defined_in", true) => "is defined in",
        ("defined_in", false) => "defines",
        ("changes", true) => "changes",
        ("changes", false) => "was changed by",
        ("verified_by", true) => "was verified by",
        ("verified_by", false) => "verified",
        ("failed", true) => "failed",
        ("failed", false) => "failed in",
        ("skipped", true) => "skipped",
        ("skipped", false) => "was skipped in",
        ("includes", true) => "ran",
        ("includes", false) => "ran in",
        ("touched", true) => "edited",
        ("touched", false) => "was edited by",
        ("attached_to", true) => "belongs to",
        ("attached_to", false) => "owns",
        ("part_of", true) => "is part of",
        ("part_of", false) => "is made of",
        ("depicts", true) => "shows",
        ("depicts", false) => "is shown by",
        ("renamed_to", true) => "moved to",
        ("renamed_to", false) => "was moved from",
        (other, true) => return other.replace('_', " "),
        (other, false) => return format!("is {} by", other.replace('_', " ")),
    };
    phrase.to_string()
}

/// The consolidation summary is written for the terminal
/// ("0 added, 5 changed file(s); 3 doc update(s); … attention: a, b, c").
/// Rewrite it as something a person would say.
pub fn session_summary(raw: &str) -> String {
    let number = |needle: &str| -> Option<u64> {
        let index = raw.find(needle)?;
        raw[..index]
            .rsplit(|c: char| !c.is_ascii_digit())
            .find(|part| !part.is_empty())
            .and_then(|digits| digits.parse().ok())
    };
    let mut parts: Vec<String> = Vec::new();
    match (number(" added"), number(" changed file")) {
        (Some(added), Some(changed)) if added + changed > 0 => parts.push(format!(
            "{} touched",
            count(added + changed, "file was", "files were")
        )),
        _ => {}
    }
    if let Some(docs) = number(" doc update").filter(|n| *n > 0) {
        parts.push(format!("{} updated", count(docs, "document was", "documents were")));
    }
    if let Some(runs) = number(" protocol").filter(|n| *n > 0) {
        parts.push(format!("{} imported", count(runs, "test run was", "test runs were")));
    }
    if let Some(notes) = number(" note").filter(|n| *n > 0) {
        parts.push(format!("{} left", count(notes, "note was", "notes were")));
    }
    if raw.contains("ok") {
        if let Some(rest) = raw.split("last run ").nth(1) {
            if let Some(verdict) = rest.split(';').next() {
                parts.push(format!("tests were {}", verdict.trim()));
            }
        }
    }
    if parts.is_empty() {
        return "the session's work was folded into memory".to_string();
    }
    format!("{} — then folded into memory", parts.join(", "))
}

/// The stage a governed change has reached, in plain words.
pub fn change_stage(status: &str) -> (&'static str, &'static str) {
    match status {
        "proposed" => ("proposed", "written down, nothing has been touched yet"),
        "applied" => ("applied", "the file was written; not verified yet"),
        "verified" => ("verified", "applied and the tests passed"),
        "broken" => ("broken", "applied, but the tests failed"),
        "reverted" => ("reverted", "undone; the file is back to how it was"),
        "failed" => ("failed", "the write did not succeed"),
        "indeterminate" => (
            "unknown",
            "the process stopped mid-change — nobody knows if the write landed",
        ),
        _ => ("recorded", ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_reads_like_speech() {
        let now = 1_000_000_000;
        assert_eq!(ago(now, now - 10_000), "just now");
        assert_eq!(ago(now, now - 600_000), "10 minutes ago");
        assert_eq!(ago(now, now - 3 * 3_600_000), "3 hours ago");
        assert_eq!(ago(now, now - 25 * 3_600_000), "yesterday");
        assert_eq!(ago(now, now - 3 * 86_400_000), "3 days ago");
        assert_eq!(ago(now, 0), "at an unrecorded time");
    }

    #[test]
    fn attention_reasons_lose_their_jargon() {
        assert_eq!(
            attention_reason("hub 29").unwrap(),
            "29 files import this — edits here ripple widest"
        );
        assert_eq!(attention_reason("untested hub").unwrap(), "no test covers it");
        // Nothing recent is nothing to say.
        assert_eq!(attention_reason("churn 2 (0 recent)"), None);
        assert_eq!(
            attention_reason("churn 9 (0 recent)").unwrap(),
            "changed 9 times over its history"
        );
        assert_eq!(
            attention_reason("churn 4 (2 recent)").unwrap(),
            "changed 2 times since your last session"
        );
        assert_eq!(
            attention_reason("churn 4 (1 recent)").unwrap(),
            "changed once since your last session"
        );
        assert_eq!(
            attention_reason("2 failing test(s)").unwrap(),
            "2 tests are failing"
        );
    }

    #[test]
    fn states_explain_themselves() {
        assert_eq!(lifecycle("active", ""), None);
        assert_eq!(
            lifecycle("superseded", "superseded by adr-014").unwrap(),
            "superseded — replaced by adr-014"
        );
        assert_eq!(
            lifecycle("retired", "source file deleted").unwrap(),
            "retired — the file it lived in was deleted"
        );
        let (state, why) = freshness(Some("warn"), true);
        assert_eq!(state, "may be wrong");
        assert!(why.contains("changed after"));
        assert_eq!(freshness(Some("warn"), false).0, "historical");
    }

    #[test]
    fn predicates_become_phrases() {
        assert_eq!(predicate_phrase("imports", true), "uses");
        assert_eq!(predicate_phrase("imports", false), "is used by");
        assert_eq!(predicate_phrase("covers", false), "is tested by");
        assert_eq!(predicate_phrase("weird_edge", true), "weird edge");
    }

    /// Every edge a feature or a run writes is read from both ends. The
    /// generic fall-through produces "is implemented by by", which is what
    /// an implementation file used to say about the feature claiming it.
    #[test]
    fn an_edge_reads_from_both_ends() {
        for predicate in [
            "implemented_by",
            "tested_by",
            "decided_by",
            "documented_in",
            "verified_by",
            "failed",
            "skipped",
            "includes",
            "touched",
        ] {
            for outgoing in [true, false] {
                let phrase = predicate_phrase(predicate, outgoing);
                assert!(
                    !phrase.contains("by by") && !phrase.contains('_'),
                    "{predicate} (outgoing={outgoing}) reads as {phrase:?}"
                );
            }
        }
    }
}
