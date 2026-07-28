//! Tests and test protocols in the graph.
//!
//! Two halves, sharing the twin's identity scheme:
//!
//! - **Static** (refresh-time, zero-config): twinned files are classified
//!   as tests by framework — Rust `#[test]`, Playwright/Jest specs, pytest
//!   files, PHPUnit classes — with `test_framework` / `tests_declared` /
//!   `file_role` observations, and `covers` relations from a test file to
//!   the source files it imports.
//! - **Dynamic** (protocols): a test run report — `cargo test` output,
//!   JUnit XML (the interchange format Playwright, pytest, PHPUnit and
//!   Jest all emit), or Playwright's own JSON — is imported as a
//!   content-addressed `test_run` entity. Each test case is an entity
//!   whose `result` observations are guarded: the timeline records
//!   *transitions* (pass→fail, fail→pass), which is exactly the
//!   flake/regression history. A run also writes Evidence (Behavioral) on
//!   the repo entity, tying protocols into the verification taxonomy.
//!
//! Playwright's JSON is parsed separately from its JUnit export because
//! it is the only report that names the **attachments** a run produced —
//! the failure screenshot, the video, the trace. Those become declared
//! assets owned by the case that produced them, so a browser failure is
//! something you look at, not just a name in a list.

use crate::twin::{latest, observe_src, relate};
use brain_core::ids::StableId;
use brain_core::object::{Object, VerificationLevel};
use brain_index::{replay, Index, MemIndex};
use brain_store::{now_ms, Store, StoreError};
use std::collections::{BTreeMap, BTreeSet};

// ---------------------------------------------------------------------------
// Static classification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct TestInfo {
    /// rust | playwright | jest | pytest | phpunit
    pub framework: &'static str,
    /// Test cases declared in the file (best-effort count).
    pub declared: usize,
    /// True when the whole file is a test (by convention); false for
    /// source files with inline tests (Rust `#[cfg(test)]`).
    pub is_test_file: bool,
}

/// Classify a twinned file as test code, if it is any.
pub fn classify(rel_path: &str, language: &str, content: &str) -> Option<TestInfo> {
    let file = rel_path.rsplit('/').next().unwrap_or(rel_path);
    match language {
        "rust" => {
            let declared = content
                .lines()
                .filter(|l| {
                    let t = l.trim();
                    t == "#[test]" || t.starts_with("#[test(") || t.ends_with("::test]")
                })
                .count();
            if declared == 0 {
                return None;
            }
            let by_convention = rel_path.starts_with("tests/") || rel_path.contains("/tests/");
            Some(TestInfo {
                framework: "rust",
                declared,
                is_test_file: by_convention,
            })
        }
        "javascript" => {
            let is_spec = file.contains(".test.") || file.contains(".spec.");
            let playwright = content.contains("@playwright/test");
            let declared = content
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    t.starts_with("test(")
                        || t.starts_with("test.only(")
                        || t.starts_with("it(")
                        || t.starts_with("it.only(")
                })
                .count();
            if !is_spec && !playwright && declared == 0 {
                return None;
            }
            let framework = if playwright { "playwright" } else { "jest" };
            Some(TestInfo {
                framework,
                declared,
                is_test_file: is_spec || playwright,
            })
        }
        "python" => {
            let is_test = file.starts_with("test_") || file.ends_with("_test.py");
            let declared = content
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    t.starts_with("def test_") || t.starts_with("async def test_")
                })
                .count();
            if !is_test && declared == 0 {
                return None;
            }
            Some(TestInfo {
                framework: "pytest",
                declared,
                is_test_file: is_test,
            })
        }
        "php" => {
            let is_test = file.ends_with("Test.php");
            let declared = content
                .lines()
                .filter(|l| l.trim_start().starts_with("public function test"))
                .count();
            if !is_test && declared == 0 {
                return None;
            }
            Some(TestInfo {
                framework: "phpunit",
                declared,
                is_test_file: is_test,
            })
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Run report parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseStatus {
    Pass,
    Fail,
    Skip,
}

impl CaseStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CaseStatus::Pass => "pass",
            CaseStatus::Fail => "fail",
            CaseStatus::Skip => "skip",
        }
    }
}

/// A file a test run produced about a case: a screenshot, a video, a
/// trace. Playwright is the only reporter that names these; the path is
/// as the reporter wrote it and is made workspace-relative at record time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub name: String,
    pub path: String,
    pub content_type: String,
}

/// One case in a run report.
///
/// `cargo` and JUnit fill only `name` and `status` — that is all their
/// formats carry. The Playwright JSON reporter fills every field, which is
/// why it is worth parsing separately: it is the only format that names
/// the spec file directly, keeps the failure message, counts retries, and
/// lists the screenshots and videos the run produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    pub name: String,
    pub status: CaseStatus,
    pub duration_ms: Option<u64>,
    /// First line of the failure message, when the reporter gave one.
    pub error: Option<String>,
    /// The spec file, when the reporter names it rather than encoding it
    /// in the case name.
    pub file: Option<String>,
    pub line: Option<u32>,
    pub retries: usize,
    pub attachments: Vec<Attachment>,
}

impl Case {
    /// A case with only what the textual formats can tell us.
    pub fn plain(name: String, status: CaseStatus) -> Self {
        Case {
            name,
            status,
            duration_ms: None,
            error: None,
            file: None,
            line: None,
            retries: 0,
            attachments: Vec::new(),
        }
    }
}

#[derive(Debug, Default)]
pub struct RunReport {
    pub format: &'static str,
    pub cases: Vec<Case>,
    /// Wall time for the whole run, when the reporter states it.
    pub total_duration_ms: Option<u64>,
}

impl RunReport {
    fn count(&self, want: CaseStatus) -> usize {
        self.cases.iter().filter(|c| c.status == want).count()
    }
    pub fn passed(&self) -> usize {
        self.count(CaseStatus::Pass)
    }
    pub fn failed(&self) -> usize {
        self.count(CaseStatus::Fail)
    }
    pub fn skipped(&self) -> usize {
        self.count(CaseStatus::Skip)
    }
}

/// Parse a test report, auto-detecting the format.
pub fn parse_report(text: &str) -> RunReport {
    let head = text.trim_start();
    if head.starts_with('{') && head.contains("\"suites\"") {
        parse_playwright_json(text)
    } else if text.contains("<testcase") || text.contains("<testsuite") {
        parse_junit(text)
    } else {
        parse_cargo(text)
    }
}

/// `cargo test` textual output: `test path::name ... ok|FAILED|ignored`.
pub fn parse_cargo(text: &str) -> RunReport {
    let mut report = RunReport {
        format: "cargo",
        ..RunReport::default()
    };
    for line in text.lines() {
        let t = line.trim();
        let Some(rest) = t.strip_prefix("test ") else {
            continue;
        };
        if rest.starts_with("result:") {
            continue;
        }
        let Some((name, verdict)) = rest.rsplit_once(" ... ") else {
            continue;
        };
        let status = match verdict.split_whitespace().next() {
            Some("ok") => CaseStatus::Pass,
            Some("FAILED") => CaseStatus::Fail,
            Some("ignored") => CaseStatus::Skip,
            _ => continue,
        };
        report
            .cases
            .push(Case::plain(name.trim().to_string(), status));
    }
    report
}

/// JUnit XML, best-effort line-free scan: every framework's interchange
/// format (Playwright, pytest, PHPUnit, Jest all export it). A testcase's
/// name is `classname::name`; Playwright puts the spec file path in
/// `classname`, which lets test cases link back to their twinned file.
pub fn parse_junit(text: &str) -> RunReport {
    let mut report = RunReport {
        format: "junit",
        ..RunReport::default()
    };
    for chunk in text.split("<testcase").skip(1) {
        // A testcase's scope ends at its closing tag when it has children,
        // else at the next testcase; <failure>/<error>/<skipped> children
        // live inside that scope.
        let scope = match chunk.split_once("</testcase>") {
            Some((inner, _)) => inner,
            None => chunk,
        };
        let name = xml_attr(scope, "name").unwrap_or_default();
        let classname = xml_attr(scope, "classname").unwrap_or_default();
        let full = if classname.is_empty() || classname == name {
            name.clone()
        } else {
            format!("{classname}::{name}")
        };
        if full.is_empty() {
            continue;
        }
        let status = if scope.contains("<failure") || scope.contains("<error") {
            CaseStatus::Fail
        } else if scope.contains("<skipped") {
            CaseStatus::Skip
        } else {
            CaseStatus::Pass
        };
        let mut case = Case::plain(full, status);
        // `time` is seconds with a fraction; the graph keeps milliseconds.
        case.duration_ms = xml_attr(scope, "time")
            .and_then(|t| t.parse::<f64>().ok())
            .map(|secs| (secs * 1000.0).round() as u64);
        if status == CaseStatus::Fail {
            case.error = xml_attr(scope, "message").map(|m| first_line(&xml_unescape(&m)));
        }
        // Playwright's JUnit export puts the spec path in `classname`; when
        // it looks like a path, it is the file, not a namespace.
        if classname.contains('/') {
            case.file = Some(classname);
        }
        report.cases.push(case);
    }
    report
}

fn first_line(text: &str) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let line = line.trim();
    if line.chars().count() > 240 {
        let cut = line
            .char_indices()
            .nth(240)
            .map(|(i, _)| i)
            .unwrap_or(line.len());
        format!("{}…", &line[..cut])
    } else {
        line.to_string()
    }
}

fn xml_unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#10;", "\n")
        .replace("&#13;", "")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Playwright's JSON reporter (`--reporter=json`).
///
/// JUnit is the interchange format every framework emits, and it is what
/// `parse_junit` handles. Playwright's own JSON is worth a separate parser
/// for one reason: it lists the **attachments** a run produced — the
/// failure screenshot, the video, the trace — which is the evidence a
/// person actually wants when a browser test fails. It also names the spec
/// file directly, keeps the error message, and counts retries.
///
/// Shape: `{"suites": [{"file", "title", "specs": [{"title", "line",
/// "tests": [{"projectName", "results": [{"status", "duration", "retry",
/// "error", "attachments"}]}]}], "suites": [...nested]}]}`.
pub fn parse_playwright_json(text: &str) -> RunReport {
    let mut report = RunReport {
        format: "playwright",
        ..RunReport::default()
    };
    let Ok(root) = serde_json::from_str::<serde_json::Value>(text) else {
        return report;
    };
    report.total_duration_ms = root
        .get("stats")
        .and_then(|s| s.get("duration"))
        .and_then(|d| d.as_f64())
        .map(|ms| ms.round() as u64);
    if let Some(suites) = root.get("suites").and_then(|s| s.as_array()) {
        for suite in suites {
            walk_playwright_suite(suite, &[], &mut report.cases);
        }
    }
    report
}

fn walk_playwright_suite(suite: &serde_json::Value, titles: &[String], out: &mut Vec<Case>) {
    let title = suite
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or_default();
    let file = suite.get("file").and_then(|f| f.as_str());
    // The outermost suite's title *is* the file path; deeper suites are
    // `describe` blocks and belong in the case name.
    let mut path = titles.to_vec();
    if !title.is_empty() && Some(title) != file {
        path.push(title.to_string());
    }

    for spec in suite
        .get("specs")
        .and_then(|s| s.as_array())
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let spec_file = spec
            .get("file")
            .and_then(|f| f.as_str())
            .or(file)
            .map(str::to_string);
        let spec_title = spec
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or_default();
        let line = spec
            .get("line")
            .and_then(|l| l.as_u64())
            .map(|l| l as u32);

        for test in spec
            .get("tests")
            .and_then(|t| t.as_array())
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let Some(case) = playwright_case(test, &path, spec_title, &spec_file, line) else {
                continue;
            };
            out.push(case);
        }
    }

    for nested in suite
        .get("suites")
        .and_then(|s| s.as_array())
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        walk_playwright_suite(nested, &path, out);
    }
}

fn playwright_case(
    test: &serde_json::Value,
    describes: &[String],
    spec_title: &str,
    file: &Option<String>,
    line: Option<u32>,
) -> Option<Case> {
    let results = test.get("results")?.as_array()?;
    // The last attempt is the verdict; earlier ones are the retries.
    let last = results.last()?;
    let status = match last.get("status").and_then(|s| s.as_str()).unwrap_or("") {
        "passed" => CaseStatus::Pass,
        "skipped" => CaseStatus::Skip,
        // failed, timedOut, interrupted
        "" => return None,
        _ => CaseStatus::Fail,
    };

    let mut title = describes.to_vec();
    title.push(spec_title.to_string());
    let mut name = title.join(" › ");
    if let Some(project) = test
        .get("projectName")
        .and_then(|p| p.as_str())
        .filter(|p| !p.is_empty())
    {
        name = format!("{name} [{project}]");
    }
    // `file::name` is the shape every other reporter produces, and the
    // shape `defined_in` linkage already understands.
    if let Some(file) = file {
        name = format!("{file}::{name}");
    }

    let mut case = Case::plain(name, status);
    case.file = file.clone();
    case.line = line;
    case.retries = results.len().saturating_sub(1);
    case.duration_ms = last
        .get("duration")
        .and_then(|d| d.as_f64())
        .map(|ms| ms.round() as u64);
    case.error = last
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .map(|m| first_line(&strip_ansi(m)));

    // Attachments from every attempt: when a test is flaky, the failing
    // attempt's screenshot is the one worth keeping.
    let mut seen = BTreeSet::new();
    for result in results {
        for att in result
            .get("attachments")
            .and_then(|a| a.as_array())
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let Some(path) = att.get("path").and_then(|p| p.as_str()) else {
                continue; // inline `body` attachments have no file
            };
            if !seen.insert(path.to_string()) {
                continue;
            }
            case.attachments.push(Attachment {
                name: att
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("attachment")
                    .to_string(),
                path: path.to_string(),
                content_type: att
                    .get("contentType")
                    .and_then(|c| c.as_str())
                    .unwrap_or_default()
                    .to_string(),
            });
        }
    }
    Some(case)
}

/// Playwright colours its error messages; the graph stores plain text.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        // CSI ... final byte in @-~
        for c in chars.by_ref() {
            if ('\u{40}'..='\u{7e}').contains(&c) && c != '[' {
                break;
            }
        }
    }
    out
}

fn xml_attr(chunk: &str, attr: &str) -> Option<String> {
    // The leading space keeps `name=` from matching inside `classname=`.
    let needle = format!(" {attr}=\"");
    let start = chunk.find(&needle)? + needle.len();
    let end = chunk[start..].find('"')?;
    Some(chunk[start..start + end].to_string())
}

// ---------------------------------------------------------------------------
// Recording a run into the graph
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct RunOutcome {
    pub run_sid: StableId,
    /// False when this exact report was already imported (idempotent).
    pub wrote: bool,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    /// Names of failing cases in this report.
    pub failing: Vec<String>,
    /// Result flips recorded (pass->fail / fail->pass): the flake signal.
    pub transitions: usize,
}

/// Import a test run. The run's identity is the content hash of the raw
/// report, so re-importing the same report writes nothing. Per-case
/// `result` observations are guarded: only transitions are recorded.
pub fn record_run(
    store: &Store,
    prefix: &str,
    report: &RunReport,
    raw: &str,
) -> Result<RunOutcome, StoreError> {
    record_run_in(store, std::path::Path::new("."), prefix, report, raw)
}

/// `record_run` with the workspace root, which is what lets a reporter's
/// attachment paths be resolved to workspace-relative files and declared
/// as assets.
pub fn record_run_in(
    store: &Store,
    root: &std::path::Path,
    prefix: &str,
    report: &RunReport,
    raw: &str,
) -> Result<RunOutcome, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    let now = now_ms();
    let hash = blake3::hash(raw.as_bytes()).to_hex().to_string();
    let run_sid = StableId::derive(&["test_run", prefix, &hash]);

    let mut outcome = RunOutcome {
        run_sid: run_sid.clone(),
        wrote: false,
        total: report.cases.len(),
        passed: report.passed(),
        failed: report.failed(),
        skipped: report.skipped(),
        failing: report
            .cases
            .iter()
            .filter(|c| c.status == CaseStatus::Fail)
            .map(|c| c.name.clone())
            .collect(),
        transitions: 0,
    };
    if !index.entity_nodes(&run_sid).is_empty() {
        return Ok(outcome); // this exact report is already in the graph
    }
    outcome.wrote = true;

    let mut labels = BTreeMap::new();
    labels.insert("prefix".to_string(), prefix.to_string());
    labels.insert("format".to_string(), report.format.to_string());
    labels.insert("report_b3".to_string(), hash.clone());
    store.put(&Object::Entity {
        id: run_sid.clone(),
        entity_kind: "test_run".to_string(),
        labels,
    })?;
    for (prop, value) in [
        ("total", outcome.total),
        ("passed", outcome.passed),
        ("failed", outcome.failed),
        ("skipped", outcome.skipped),
    ] {
        observe_src(store, &run_sid, prop, &value.to_string(), "testrun", now)?;
    }
    if let Some(ms) = report.total_duration_ms {
        observe_src(store, &run_sid, "duration_ms", &ms.to_string(), "testrun", now)?;
    }

    let repo_sid = StableId::derive(&["repo", prefix]);
    let mut written = BTreeSet::new();
    relate(
        store,
        &index,
        &mut written,
        &run_sid,
        "concerns",
        &repo_sid,
        now,
    )?;

    let files = crate::twin::twinned_paths(store, prefix)?;
    let workspace = root.canonicalize().ok();
    let mut asset_edges: BTreeSet<(StableId, String, StableId)> = BTreeSet::new();
    for case in &report.cases {
        let name = &case.name;
        let status = case.status;
        let case_sid = StableId::derive(&["test", prefix, name]);
        let mut labels = BTreeMap::new();
        labels.insert("prefix".to_string(), prefix.to_string());
        labels.insert("name".to_string(), name.clone());
        store.put(&Object::Entity {
            id: case_sid.clone(),
            entity_kind: "test_case".to_string(),
            labels,
        })?;
        let prior = latest(&index, store, &case_sid, "result")?;
        let flipped = prior.is_some() && prior.as_deref() != Some(status.as_str());
        if prior.as_deref() != Some(status.as_str()) {
            observe_src(store, &case_sid, "result", status.as_str(), "testrun", now)?;
            if flipped {
                outcome.transitions += 1;
            }
        }

        // Detail the textual formats cannot carry. Guarded like every
        // other observation, so a stable test writes nothing on re-import.
        for (prop, value) in [
            ("duration_ms", case.duration_ms.map(|d| d.to_string())),
            ("error", case.error.clone()),
            (
                "retries",
                (case.retries > 0).then(|| case.retries.to_string()),
            ),
            ("line", case.line.map(|l| l.to_string())),
        ] {
            let Some(value) = value else { continue };
            if latest(&index, store, &case_sid, prop)?.as_deref() != Some(value.as_str()) {
                observe_src(store, &case_sid, prop, &value, "testrun", now)?;
            }
        }

        if status == CaseStatus::Fail {
            relate(
                store,
                &index,
                &mut written,
                &run_sid,
                "failed",
                &case_sid,
                now,
            )?;
        }
        if status == CaseStatus::Skip {
            relate(
                store,
                &index,
                &mut written,
                &run_sid,
                "skipped",
                &case_sid,
                now,
            )?;
        }
        // Run membership is recorded only where it carries information:
        // what failed, what was skipped, what changed its mind. Linking
        // every passing case to every run would add one edge per case per
        // import for a fact the case's own result timeline already states.
        if status != CaseStatus::Pass || flipped {
            relate(
                store,
                &index,
                &mut written,
                &run_sid,
                "includes",
                &case_sid,
                now,
            )?;
        }

        // Where the case lives. Playwright names the spec file outright;
        // JUnit encodes it as the classname before `::`.
        let declared_file = case
            .file
            .as_deref()
            .filter(|f| files.contains(*f))
            .or_else(|| name.split_once("::").map(|(c, _)| c).filter(|c| files.contains(*c)));
        if let Some(file) = declared_file {
            let file_sid = StableId::derive(&["file", file]);
            relate(
                store,
                &index,
                &mut written,
                &case_sid,
                "defined_in",
                &file_sid,
                now,
            )?;
        }

        // What the run produced about this case: the failure screenshot,
        // the video, the trace. These are the evidence a person opens
        // first, so they become declared assets owned by the case rather
        // than anonymous files under test-results/.
        for attachment in &case.attachments {
            let Some(rel) = workspace
                .as_ref()
                .and_then(|root| workspace_relative(root, &attachment.path))
            else {
                continue;
            };
            let depicts: Vec<StableId> = declared_file
                .map(|f| vec![StableId::derive(&["file", f])])
                .unwrap_or_default();
            crate::assets::declare(
                store,
                &index,
                &mut asset_edges,
                prefix,
                &rel,
                &case_sid,
                &depicts,
                Some(attachment_subtype(attachment)),
                now,
            )?;
        }
    }

    // The protocol as Evidence: a Behavioral claim about the repo entity.
    // (A run imported before any refresh has no repo entity yet; evidence
    // is skipped then rather than invented.)
    if let Some(repo_node) = index.entity_nodes(&repo_sid).first().copied() {
        store.put(&Object::Evidence {
            subject: repo_node,
            level: VerificationLevel::Behavioral,
            method: format!("testrun@{}", &hash[..12]),
            passed: outcome.failed == 0,
            detail: format!(
                "{} passed, {} failed, {} skipped ({})",
                outcome.passed, outcome.failed, outcome.skipped, report.format
            ),
        })?;
    }
    Ok(outcome)
}

/// Resolve a reporter's attachment path against the workspace.
///
/// Playwright writes absolute paths. A path that does not exist, or that
/// resolves outside the workspace, is dropped rather than recorded — the
/// graph never names bytes it cannot point at.
fn workspace_relative(root: &std::path::Path, path: &str) -> Option<String> {
    let candidate = std::path::Path::new(path);
    let absolute = if candidate.is_absolute() {
        candidate.canonicalize().ok()?
    } else {
        root.join(candidate).canonicalize().ok()?
    };
    let rel = absolute.strip_prefix(root).ok()?;
    let rel = rel.to_str()?.replace('\\', "/");
    (!rel.is_empty()).then_some(rel)
}

fn attachment_subtype(attachment: &Attachment) -> &'static str {
    if attachment.content_type.starts_with("image/") {
        "image"
    } else if attachment.content_type.starts_with("video/") {
        "screencast"
    } else if attachment.content_type.starts_with("audio/") {
        "audio"
    } else if attachment.name == "trace" || attachment.path.ends_with(".zip") {
        "trace"
    } else {
        crate::assets::infer_subtype(&attachment.path)
    }
}

/// Test runs under a prefix, newest first: (at_ms, total, passed, failed,
/// format). Ordered by the event log, not timestamp sorting, so runs
/// imported in the same millisecond keep their true order.
pub fn runs(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
) -> Result<Vec<(u64, usize, usize, usize, String)>, StoreError> {
    // The store already holds this map, parsed once per graph version.
    let positions = store.put_position()?;
    let mut seen: BTreeSet<StableId> = BTreeSet::new();
    let mut out = Vec::new();
    for node in index.entities_by_kind("test_run") {
        let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
            continue;
        };
        if labels.get("prefix").map(String::as_str) != Some(prefix) || !seen.insert(id.clone()) {
            continue;
        }
        let Some((at, total)) = crate::twin::latest_at(index, store, &id, "total")? else {
            continue;
        };
        let pos = index
            .observations_of(&id)
            .iter()
            .filter_map(|n| positions.get(n))
            .max()
            .copied()
            .unwrap_or(0);
        let passed = latest(index, store, &id, "passed")?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let failed = latest(index, store, &id, "failed")?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let format = labels.get("format").cloned().unwrap_or_default();
        out.push((pos, at, total.parse().unwrap_or(0), passed, failed, format));
    }
    out.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(out
        .into_iter()
        .map(|(_, at, t, p, f, fmt)| (at, t, p, f, fmt))
        .collect())
}

/// Test cases whose latest recorded result is `fail`, sorted by name.
pub fn failing_cases(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
) -> Result<Vec<String>, StoreError> {
    let mut seen: BTreeSet<StableId> = BTreeSet::new();
    let mut out = Vec::new();
    for node in index.entities_by_kind("test_case") {
        let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
            continue;
        };
        if labels.get("prefix").map(String::as_str) != Some(prefix) || !seen.insert(id.clone()) {
            continue;
        }
        if latest(index, store, &id, "result")?.as_deref() == Some("fail") {
            out.push(labels.get("name").cloned().unwrap_or_default());
        }
    }
    out.sort();
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
    fn classification_by_framework() {
        let rust = classify("src/lib.rs", "rust", "pub fn a() {}\n#[test]\nfn t() {}\n").unwrap();
        assert_eq!(
            (rust.framework, rust.declared, rust.is_test_file),
            ("rust", 1, false)
        );
        let rust = classify(
            "tests/it.rs",
            "rust",
            "#[test]\nfn t() {}\n#[tokio::test]\nasync fn u() {}\n",
        )
        .unwrap();
        assert_eq!((rust.declared, rust.is_test_file), (2, true));
        assert!(classify("src/lib.rs", "rust", "pub fn a() {}\n").is_none());

        let pw = classify(
            "e2e/login.spec.ts",
            "javascript",
            "import { test } from '@playwright/test';\ntest('logs in', async () => {});\n",
        )
        .unwrap();
        assert_eq!(
            (pw.framework, pw.declared, pw.is_test_file),
            ("playwright", 1, true)
        );
        let jest = classify(
            "web/app.test.js",
            "javascript",
            "import { render } from './app';\ntest('renders', () => {});\nit('updates', () => {});\n",
        )
        .unwrap();
        assert_eq!((jest.framework, jest.declared), ("jest", 2));

        let py = classify("test_cli.py", "python", "def test_main():\n    pass\n").unwrap();
        assert_eq!(
            (py.framework, py.declared, py.is_test_file),
            ("pytest", 1, true)
        );
        let php = classify(
            "tests/UserTest.php",
            "php",
            "<?php\nclass UserTest {\npublic function testLoad() {}\n}\n",
        )
        .unwrap();
        assert_eq!(
            (php.framework, php.declared, php.is_test_file),
            ("phpunit", 1, true)
        );
    }

    #[test]
    fn cargo_report_parses_cases_and_totals() {
        let out = "running 3 tests\n\
                   test twin::tests::a ... ok\n\
                   test twin::tests::b ... FAILED\n\
                   test twin::tests::c ... ignored\n\
                   test result: FAILED. 1 passed; 1 failed; 1 ignored\n";
        let r = parse_report(out);
        assert_eq!(r.format, "cargo");
        assert_eq!(r.cases.len(), 3);
        assert_eq!((r.passed(), r.failed(), r.skipped()), (1, 1, 1));
        assert_eq!(r.cases[1].name, "twin::tests::b");
        assert_eq!(r.cases[1].status, CaseStatus::Fail);
        // cargo says nothing beyond the verdict, and we invent nothing.
        assert_eq!(r.cases[1].duration_ms, None);
        assert_eq!(r.cases[1].error, None);
        assert!(r.cases[1].attachments.is_empty());
    }

    #[test]
    fn junit_report_parses_playwright_style() {
        let xml = r#"<?xml version="1.0"?>
<testsuites>
 <testsuite name="login" tests="3">
  <testcase classname="e2e/login.spec.ts" name="logs in" time="0.5"/>
  <testcase classname="e2e/login.spec.ts" name="rejects bad password" time="0.4">
    <failure message="expected 401">boom</failure>
  </testcase>
  <testcase classname="e2e/login.spec.ts" name="remembers me"><skipped/></testcase>
 </testsuite>
</testsuites>
"#;
        let r = parse_report(xml);
        assert_eq!(r.format, "junit");
        assert_eq!(r.cases.len(), 3);
        assert_eq!((r.passed(), r.failed(), r.skipped()), (1, 1, 1));
        assert_eq!(r.cases[0].name, "e2e/login.spec.ts::logs in");
        assert_eq!(r.cases[1].status, CaseStatus::Fail);
        // JUnit does carry a duration and a failure message; keep them.
        assert_eq!(r.cases[0].duration_ms, Some(500));
        assert_eq!(r.cases[1].error.as_deref(), Some("expected 401"));
        assert_eq!(r.cases[0].file.as_deref(), Some("e2e/login.spec.ts"));
    }

    #[test]
    fn playwright_json_keeps_the_evidence_junit_throws_away() {
        let json = r#"{
          "stats": { "duration": 4210.5 },
          "suites": [{
            "title": "e2e/login.spec.ts",
            "file": "e2e/login.spec.ts",
            "specs": [
              { "title": "logs in", "file": "e2e/login.spec.ts", "line": 7,
                "tests": [{ "projectName": "chromium", "results": [
                  { "status": "passed", "duration": 812, "attachments": [] }
                ]}]
              },
              { "title": "rejects a bad password", "file": "e2e/login.spec.ts", "line": 21,
                "tests": [{ "projectName": "chromium", "results": [
                  { "status": "failed", "duration": 1900, "retry": 0,
                    "error": { "message": "\u001b[31mExpected 401\u001b[39m\nat login.spec.ts:24" },
                    "attachments": [
                      { "name": "screenshot", "path": "test-results/login-rejects/test-failed-1.png", "contentType": "image/png" },
                      { "name": "video", "path": "test-results/login-rejects/video.webm", "contentType": "video/webm" }
                    ]},
                  { "status": "failed", "duration": 1750, "retry": 1,
                    "error": { "message": "Expected 401" },
                    "attachments": [
                      { "name": "screenshot", "path": "test-results/login-rejects/retry1/test-failed-1.png", "contentType": "image/png" }
                    ]}
                ]}]
              }
            ],
            "suites": [{
              "title": "when remembered",
              "specs": [{ "title": "skips the form", "file": "e2e/login.spec.ts", "line": 40,
                "tests": [{ "projectName": "chromium", "results": [{ "status": "skipped" }] }]
              }]
            }]
          }]
        }"#;
        let r = parse_report(json);
        assert_eq!(r.format, "playwright");
        assert_eq!(r.total_duration_ms, Some(4211));
        assert_eq!((r.passed(), r.failed(), r.skipped()), (1, 1, 1));

        let failing = r.cases.iter().find(|c| c.status == CaseStatus::Fail).unwrap();
        assert_eq!(
            failing.name,
            "e2e/login.spec.ts::rejects a bad password [chromium]"
        );
        assert_eq!(failing.file.as_deref(), Some("e2e/login.spec.ts"));
        assert_eq!(failing.line, Some(21));
        assert_eq!(failing.retries, 1, "two attempts is one retry");
        assert_eq!(failing.duration_ms, Some(1750), "the last attempt decides");
        assert_eq!(
            failing.error.as_deref(),
            Some("Expected 401"),
            "colour codes stripped, first line only"
        );
        assert_eq!(
            failing.attachments.len(),
            3,
            "every attempt's evidence is kept, deduplicated by path"
        );

        // A nested describe block becomes part of the name, not a lost suite.
        let skipped = r.cases.iter().find(|c| c.status == CaseStatus::Skip).unwrap();
        assert_eq!(
            skipped.name,
            "e2e/login.spec.ts::when remembered › skips the form [chromium]"
        );
    }

    #[test]
    fn a_playwright_run_links_screenshots_to_the_case_that_produced_them() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("e2e")).unwrap();
        std::fs::create_dir_all(root.join("test-results/login")).unwrap();
        std::fs::write(
            root.join("e2e/login.spec.ts"),
            "import { test } from '@playwright/test';\ntest('rejects', async () => {});\n",
        )
        .unwrap();
        std::fs::write(root.join("test-results/login/test-failed-1.png"), b"\x89PNG").unwrap();

        let store = Store::open(&root.join(".brain")).unwrap();
        crate::twin::refresh(&store, root, "twin/app").unwrap();

        let json = r#"{"suites":[{"title":"e2e/login.spec.ts","file":"e2e/login.spec.ts",
          "specs":[{"title":"rejects","file":"e2e/login.spec.ts","line":2,
            "tests":[{"projectName":"","results":[{"status":"failed","duration":9,
              "error":{"message":"nope"},
              "attachments":[{"name":"screenshot","path":"test-results/login/test-failed-1.png","contentType":"image/png"}]}]}]}]}]}"#;
        let report = parse_report(json);
        let out = record_run_in(&store, root, "twin/app", &report, json).unwrap();
        assert!(out.wrote);
        assert_eq!((out.total, out.failed), (1, 1));

        let index = fresh_index(&store);
        let case_sid = StableId::derive(&["test", "twin/app", "e2e/login.spec.ts::rejects"]);

        // The detail JUnit and cargo cannot carry.
        assert_eq!(
            latest(&index, &store, &case_sid, "error").unwrap().as_deref(),
            Some("nope")
        );
        assert_eq!(
            latest(&index, &store, &case_sid, "duration_ms")
                .unwrap()
                .as_deref(),
            Some("9")
        );
        assert_eq!(
            latest(&index, &store, &case_sid, "result").unwrap().as_deref(),
            Some("fail")
        );

        // The case knows where it lives, and the failure screenshot is a
        // declared asset owned by it — not an anonymous file.
        let file_sid = StableId::derive(&["file", "e2e/login.spec.ts"]);
        assert!(crate::twin::live_from(&index, &store, &case_sid, "defined_in")
            .unwrap()
            .iter()
            .any(|(_, to)| *to == file_sid));

        let assets = crate::assets::list(&store, &index, "twin/app").unwrap();
        let shot = assets
            .iter()
            .find(|a| a.path == "test-results/login/test-failed-1.png")
            .expect("the screenshot is declared as an asset");
        assert_eq!(shot.subtype, "image");
        let asset_sid = crate::assets::asset_sid("twin/app", &shot.slug);
        assert!(
            crate::twin::live_from(&index, &store, &asset_sid, "attached_to")
                .unwrap()
                .iter()
                .any(|(_, to)| *to == case_sid),
            "the screenshot belongs to the case that produced it"
        );

        // Re-importing the identical report is a no-op.
        let before = store.count_objects().unwrap();
        let again = record_run_in(&store, root, "twin/app", &report, json).unwrap();
        assert!(!again.wrote);
        assert_eq!(store.count_objects().unwrap(), before);
    }
}
