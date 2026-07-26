//! Tests and test protocols in the graph.
//!
//! Two halves, sharing the twin's identity scheme:
//!
//! - **Static** (refresh-time, zero-config): twinned files are classified
//!   as tests by framework — Rust `#[test]`, Playwright/Jest specs, pytest
//!   files, PHPUnit classes — with `test_framework` / `tests_declared` /
//!   `file_role` observations, and `covers` relations from a test file to
//!   the source files it imports.
//! - **Dynamic** (protocols): a test run report (`cargo test` output or
//!   JUnit XML — the interchange format Playwright, pytest, PHPUnit and
//!   Jest all emit) is imported as a content-addressed `test_run` entity.
//!   Each test case is an entity whose `result` observations are guarded:
//!   the timeline records *transitions* (pass→fail, fail→pass), which is
//!   exactly the flake/regression history. A run also writes Evidence
//!   (Behavioral) on the repo entity, tying protocols into the
//!   verification taxonomy.

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
            let declared = content.lines().filter(|l| {
                let t = l.trim();
                t == "#[test]" || t.starts_with("#[test(") || t.ends_with("::test]")
            }).count();
            if declared == 0 {
                return None;
            }
            let by_convention = rel_path.starts_with("tests/") || rel_path.contains("/tests/");
            Some(TestInfo { framework: "rust", declared, is_test_file: by_convention })
        }
        "javascript" => {
            let is_spec = file.contains(".test.") || file.contains(".spec.");
            let playwright = content.contains("@playwright/test");
            let declared = content.lines().filter(|l| {
                let t = l.trim_start();
                t.starts_with("test(") || t.starts_with("test.only(")
                    || t.starts_with("it(") || t.starts_with("it.only(")
            }).count();
            if !is_spec && !playwright && declared == 0 {
                return None;
            }
            let framework = if playwright { "playwright" } else { "jest" };
            Some(TestInfo { framework, declared, is_test_file: is_spec || playwright })
        }
        "python" => {
            let is_test = file.starts_with("test_") || file.ends_with("_test.py");
            let declared = content.lines().filter(|l| {
                let t = l.trim_start();
                t.starts_with("def test_") || t.starts_with("async def test_")
            }).count();
            if !is_test && declared == 0 {
                return None;
            }
            Some(TestInfo { framework: "pytest", declared, is_test_file: is_test })
        }
        "php" => {
            let is_test = file.ends_with("Test.php");
            let declared = content.lines().filter(|l| {
                l.trim_start().starts_with("public function test")
            }).count();
            if !is_test && declared == 0 {
                return None;
            }
            Some(TestInfo { framework: "phpunit", declared, is_test_file: is_test })
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

#[derive(Debug, Default)]
pub struct RunReport {
    pub format: &'static str,
    pub cases: Vec<(String, CaseStatus)>,
}

impl RunReport {
    pub fn passed(&self) -> usize {
        self.cases.iter().filter(|(_, s)| *s == CaseStatus::Pass).count()
    }
    pub fn failed(&self) -> usize {
        self.cases.iter().filter(|(_, s)| *s == CaseStatus::Fail).count()
    }
    pub fn skipped(&self) -> usize {
        self.cases.iter().filter(|(_, s)| *s == CaseStatus::Skip).count()
    }
}

/// Parse a test report, auto-detecting the format.
pub fn parse_report(text: &str) -> RunReport {
    if text.contains("<testcase") || text.contains("<testsuite") {
        parse_junit(text)
    } else {
        parse_cargo(text)
    }
}

/// `cargo test` textual output: `test path::name ... ok|FAILED|ignored`.
pub fn parse_cargo(text: &str) -> RunReport {
    let mut report = RunReport { format: "cargo", cases: Vec::new() };
    for line in text.lines() {
        let t = line.trim();
        let Some(rest) = t.strip_prefix("test ") else { continue };
        if rest.starts_with("result:") {
            continue;
        }
        let Some((name, verdict)) = rest.rsplit_once(" ... ") else { continue };
        let status = match verdict.split_whitespace().next() {
            Some("ok") => CaseStatus::Pass,
            Some("FAILED") => CaseStatus::Fail,
            Some("ignored") => CaseStatus::Skip,
            _ => continue,
        };
        report.cases.push((name.trim().to_string(), status));
    }
    report
}

/// JUnit XML, best-effort line-free scan: every framework's interchange
/// format (Playwright, pytest, PHPUnit, Jest all export it). A testcase's
/// name is `classname::name`; Playwright puts the spec file path in
/// `classname`, which lets test cases link back to their twinned file.
pub fn parse_junit(text: &str) -> RunReport {
    let mut report = RunReport { format: "junit", cases: Vec::new() };
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
        report.cases.push((full, status));
    }
    report
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
            .filter(|(_, s)| *s == CaseStatus::Fail)
            .map(|(n, _)| n.clone())
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

    let repo_sid = StableId::derive(&["repo", prefix]);
    let mut written = BTreeSet::new();
    relate(store, &index, &mut written, &run_sid, "concerns", &repo_sid, now)?;

    let files = crate::twin::twinned_paths(store, prefix)?;
    for (name, status) in &report.cases {
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
        if prior.as_deref() != Some(status.as_str()) {
            observe_src(store, &case_sid, "result", status.as_str(), "testrun", now)?;
            if prior.is_some() {
                outcome.transitions += 1;
            }
        }
        if *status == CaseStatus::Fail {
            relate(store, &index, &mut written, &run_sid, "failed", &case_sid, now)?;
        }
        // JUnit classnames that are twinned file paths (Playwright's
        // convention) link the case to its file.
        if let Some((class, _)) = name.split_once("::") {
            if files.contains(class) {
                let file_sid = StableId::derive(&["file", class]);
                relate(store, &index, &mut written, &case_sid, "defined_in", &file_sid, now)?;
            }
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

/// Test runs under a prefix, newest first: (at_ms, total, passed, failed,
/// format). Ordered by the event log, not timestamp sorting, so runs
/// imported in the same millisecond keep their true order.
pub fn runs(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
) -> Result<Vec<(u64, usize, usize, usize, String)>, StoreError> {
    let positions: BTreeMap<_, _> = store
        .put_history()?
        .into_iter()
        .enumerate()
        .map(|(i, id)| (id, i))
        .collect();
    let mut seen: BTreeSet<StableId> = BTreeSet::new();
    let mut out = Vec::new();
    for node in index.entities_by_kind("test_run") {
        let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else { continue };
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
        let passed = latest(index, store, &id, "passed")?.and_then(|v| v.parse().ok()).unwrap_or(0);
        let failed = latest(index, store, &id, "failed")?.and_then(|v| v.parse().ok()).unwrap_or(0);
        let format = labels.get("format").cloned().unwrap_or_default();
        out.push((pos, at, total.parse().unwrap_or(0), passed, failed, format));
    }
    out.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(out.into_iter().map(|(_, at, t, p, f, fmt)| (at, t, p, f, fmt)).collect())
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
        let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else { continue };
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

    #[test]
    fn classification_by_framework() {
        let rust = classify("src/lib.rs", "rust", "pub fn a() {}\n#[test]\nfn t() {}\n").unwrap();
        assert_eq!((rust.framework, rust.declared, rust.is_test_file), ("rust", 1, false));
        let rust = classify("tests/it.rs", "rust", "#[test]\nfn t() {}\n#[tokio::test]\nasync fn u() {}\n").unwrap();
        assert_eq!((rust.declared, rust.is_test_file), (2, true));
        assert!(classify("src/lib.rs", "rust", "pub fn a() {}\n").is_none());

        let pw = classify(
            "e2e/login.spec.ts",
            "javascript",
            "import { test } from '@playwright/test';\ntest('logs in', async () => {});\n",
        )
        .unwrap();
        assert_eq!((pw.framework, pw.declared, pw.is_test_file), ("playwright", 1, true));
        let jest = classify(
            "web/app.test.js",
            "javascript",
            "import { render } from './app';\ntest('renders', () => {});\nit('updates', () => {});\n",
        )
        .unwrap();
        assert_eq!((jest.framework, jest.declared), ("jest", 2));

        let py = classify("test_cli.py", "python", "def test_main():\n    pass\n").unwrap();
        assert_eq!((py.framework, py.declared, py.is_test_file), ("pytest", 1, true));
        let php = classify("tests/UserTest.php", "php", "<?php\nclass UserTest {\npublic function testLoad() {}\n}\n").unwrap();
        assert_eq!((php.framework, php.declared, php.is_test_file), ("phpunit", 1, true));
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
        assert_eq!(r.cases[1], ("twin::tests::b".to_string(), CaseStatus::Fail));
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
        assert_eq!(r.cases[0].0, "e2e/login.spec.ts::logs in");
        assert_eq!(r.cases[1].1, CaseStatus::Fail);
    }
}
