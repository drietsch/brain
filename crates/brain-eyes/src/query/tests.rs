//! "Show me the tests" — all of them, with their results and their proof.
//!
//! The previous version answered with a summary: last run, failing cases,
//! flake candidates, untested hubs. Useful, but it is not what a person
//! asks for when they say *show me the tests*. This answers that literally:
//! every recorded run, every recorded case with its verdict and its
//! history, every file the twin classified as holding tests, and — where a
//! browser test produced them — the screenshot, recording and trace that
//! say what actually happened.
//!
//! Nothing here re-derives a verdict. Results come from the `result`
//! observation timeline, which is guarded, so its length *is* the flake
//! history. Attachments come from the assets a run declared.

use crate::dto::*;
use crate::query;
use crate::say;
use crate::state::Loaded;
use brain_core::ids::StableId;
use brain_index::Index;
use brain_observe::{assets, testing, twin};
use std::collections::BTreeMap;

pub fn build(loaded: &Loaded) -> Result<TestsView, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let now = loaded.snapshot.generated_at_ms;
    let insights = loaded.insights();

    let runs: Vec<TestSummary> = testing::runs(store, index, prefix)
        .map_err(|e| e.to_string())?
        .into_iter()
        .take(8)
        .map(|(at, total, passed, failed, _)| TestSummary {
            total,
            passed,
            failed,
            when: say::ago(now, at),
        })
        .collect();
    let last_run = runs.first().cloned();

    let cases = case_rows(loaded)?;
    let protocols = protocols(loaded)?;
    let suites = suites(loaded)?;

    let failing: Vec<FailingCase> = cases
        .iter()
        .filter(|case| case.result == "failing")
        .map(|case| FailingCase {
            id: case.id.clone(),
            name: case.name.clone(),
            note: case
                .error
                .clone()
                .unwrap_or_else(|| "failing in the last imported run".to_string()),
        })
        .collect();

    let mut flaky: Vec<CaseHistory> = cases
        .iter()
        .filter(|case| case.flips >= 3)
        .map(|case| CaseHistory {
            name: case.name.clone(),
            id: case.id.clone(),
            result: case.result.clone(),
            flips: case.flips,
            note: format!(
                "changed result {} — worth a look",
                say::count(case.flips as u64, "time", "times")
            ),
        })
        .collect();
    flaky.sort_by(|a, b| b.flips.cmp(&a.flips));
    flaky.truncate(8);

    let uncovered: Vec<Ref> = insights
        .untested_hubs
        .iter()
        .take(8)
        .map(|(path, _)| {
            let sid = StableId::derive(&["file", path]);
            query::make_ref(index, store, &sid)
        })
        .collect();

    let mut by_framework: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for suite in &suites {
        let entry = by_framework.entry(suite.framework.clone()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += suite.declared;
    }
    let frameworks: Vec<FrameworkCount> = by_framework
        .into_iter()
        .map(|(framework, (files, declared))| FrameworkCount {
            label: say::framework_noun(&framework).to_string(),
            framework,
            files,
            declared,
        })
        .collect();

    let headline = match &last_run {
        Some(run) if run.failed == 0 => format!("All {} tests passed {}.", run.total, run.when),
        Some(run) => format!("{} of {} tests failed {}.", run.failed, run.total, run.when),
        None => "No test run has been imported yet.".to_string(),
    };

    Ok(TestsView {
        snapshot: loaded.snapshot.clone(),
        headline,
        last_run,
        runs,
        failing,
        flaky,
        declared: insights.tests_declared,
        // Every file the twin classified, not only whole-file suites: a
        // repository with inline tests has none of the latter, and
        // "declared across 0 files" is simply false.
        files: suites.len(),
        uncovered,
        protocols,
        cases,
        suites,
        frameworks,
    })
}

/// Every recorded case, newest verdict first.
pub fn case_rows(loaded: &Loaded) -> Result<Vec<CaseRow>, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let now = loaded.snapshot.generated_at_ms;

    // Which file each classified test file belongs to, so a case can say
    // which framework produced it.
    let frameworks = framework_by_file(loaded)?;
    let attachments = attachments_by_owner(loaded)?;

    let mut out = Vec::new();
    for (sid, labels) in query::scoped(index, store, prefix, "test_case")? {
        let name = labels
            .get("name")
            .cloned()
            .unwrap_or_else(|| twin::sid_label(index, store, &sid));

        let (at_ms, raw) = twin::latest_at(index, store, &sid, "result")
            .map_err(|e| e.to_string())?
            .unwrap_or((0, String::new()));
        let (result, tone) = say::test_result(&raw);

        // The result timeline is guarded, so each entry is a change of
        // mind. Two entries mean it flipped once.
        let flips = index
            .observations_of(&sid)
            .iter()
            .filter(|node| {
                matches!(
                    store.get(node),
                    Ok(brain_core::object::Object::Observation { ref property, .. })
                        if property == "result"
                )
            })
            .count();

        let file = twin::live_from(index, store, &sid, "defined_in")
            .map_err(|e| e.to_string())?
            .first()
            .map(|(_, to)| query::make_ref(index, store, to));
        let file_path = file.as_ref().map(|r| r.label.clone());
        let framework = file_path
            .as_ref()
            .and_then(|path| frameworks.get(path).cloned());

        let text = |property: &str| -> Option<String> {
            twin::latest(index, store, &sid, property).ok().flatten()
        };
        let retries: usize = text("retries").and_then(|v| v.parse().ok()).unwrap_or(0);

        // A case name is usually `module::path::name` or `file::title`;
        // the leading part is the group a person scans by.
        let group = name
            .rsplit_once("::")
            .map(|(head, _)| head.to_string())
            .unwrap_or_else(|| "ungrouped".to_string());

        let note = if flips >= 3 {
            Some(format!(
                "changed result {} — worth a look",
                say::count(flips as u64, "time", "times")
            ))
        } else if retries > 0 {
            Some(format!(
                "passed only after {}",
                say::count(retries as u64, "retry", "retries")
            ))
        } else {
            None
        };

        out.push(CaseRow {
            id: sid.to_string(),
            name,
            group,
            result: result.to_string(),
            tone: tone.to_string(),
            when: if at_ms > 0 {
                say::ago(now, at_ms)
            } else {
                String::new()
            },
            at_ms,
            framework: framework.map(|f| say::framework_noun(&f).to_string()),
            duration: text("duration_ms")
                .and_then(|v| v.parse::<u64>().ok())
                .map(say::duration),
            error: text("error"),
            retries,
            flips,
            note,
            attachments: attachments.get(&sid).cloned().unwrap_or_default(),
            file,
        });
    }

    // Failing first — that is the only ordering anyone wants — then by
    // how recently the verdict changed.
    out.sort_by(|a, b| {
        let rank = |row: &CaseRow| match row.result.as_str() {
            "failing" => 0,
            "skipped" => 1,
            _ => 2,
        };
        rank(a)
            .cmp(&rank(b))
            .then(b.flips.cmp(&a.flips))
            .then(a.name.cmp(&b.name))
    });
    Ok(out)
}

/// Every imported run, newest first.
fn protocols(loaded: &Loaded) -> Result<Vec<Protocol>, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let now = loaded.snapshot.generated_at_ms;

    let mut out = Vec::new();
    for (sid, labels) in query::scoped(index, store, prefix, "test_run")? {
        let number = |property: &str| -> usize {
            twin::latest(index, store, &sid, property)
                .ok()
                .flatten()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0)
        };
        let at_ms = twin::latest_at(index, store, &sid, "total")
            .map_err(|e| e.to_string())?
            .map(|(at, _)| at)
            .unwrap_or(0);
        let (total, passed, failed, skipped) = (
            number("total"),
            number("passed"),
            number("failed"),
            number("skipped"),
        );

        let verdict = if total == 0 {
            "the report named no tests".to_string()
        } else if failed == 0 && skipped == 0 {
            format!("all {total} passed")
        } else if failed == 0 {
            format!("{passed} passed, {skipped} skipped")
        } else {
            format!("{failed} of {total} failed")
        };

        // Only cases the run singled out are linked; see the module note.
        let mut named = Vec::new();
        for predicate in ["failed", "skipped", "includes"] {
            for (_, to) in twin::live_from(index, store, &sid, predicate)
                .map_err(|e| e.to_string())?
            {
                if named.iter().any(|c: &CaseRef| c.id == to.to_string()) {
                    continue;
                }
                let raw = twin::latest(index, store, &to, "result")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default();
                let (result, tone) = say::test_result(&raw);
                named.push(CaseRef {
                    id: to.to_string(),
                    name: query::labels_of(index, store, &to)
                        .get("name")
                        .cloned()
                        .unwrap_or_else(|| twin::sid_label(index, store, &to)),
                    result: result.to_string(),
                    tone: tone.to_string(),
                });
            }
        }
        named.sort_by(|a, b| a.name.cmp(&b.name));

        // The Evidence object this run wrote, found by its method tag.
        let report_b3 = labels.get("report_b3").cloned().unwrap_or_default();
        let evidence = (report_b3.len() >= 12).then(|| {
            format!(
                "recorded as evidence that {}",
                say::evidence_level("behavioral")
            )
        });

        // Runs are linked from the change they verified, not to it.
        let verified_change = twin::live_to(index, store, &sid, "verified_by")
            .map_err(|e| e.to_string())?
            .first()
            .map(|(_, from)| query::make_ref(index, store, from));

        out.push(Protocol {
            id: sid.to_string(),
            when: say::ago(now, at_ms),
            at_ms,
            tone: if failed > 0 { "bad" } else { "good" }.to_string(),
            verdict,
            source: format!(
                "from {}",
                say::report_format(labels.get("format").map(String::as_str).unwrap_or(""))
            ),
            total,
            passed,
            failed,
            skipped,
            duration: twin::latest(index, store, &sid, "duration_ms")
                .map_err(|e| e.to_string())?
                .and_then(|v| v.parse::<u64>().ok())
                .map(say::duration),
            named,
            evidence,
            verified_change,
        });
    }
    out.sort_by(|a, b| b.at_ms.cmp(&a.at_ms));
    Ok(out)
}

/// Files the twin classified as holding tests.
fn suites(loaded: &Loaded) -> Result<Vec<Suite>, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();

    let mut out = Vec::new();
    for (path, sid) in query::present_files(index, store, prefix)? {
        let Some(framework) = twin::latest(index, store, &sid, "test_framework")
            .map_err(|e| e.to_string())?
        else {
            continue;
        };
        let declared: usize = twin::latest(index, store, &sid, "tests_declared")
            .map_err(|e| e.to_string())?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let whole_file = twin::latest(index, store, &sid, "file_role")
            .map_err(|e| e.to_string())?
            .as_deref()
            == Some("test");
        let covers: Vec<Ref> = twin::live_from(index, store, &sid, "covers")
            .map_err(|e| e.to_string())?
            .iter()
            .take(6)
            .map(|(_, to)| query::make_ref(index, store, to))
            .collect();

        let note = if whole_file {
            format!(
                "a {} test file",
                say::framework_noun(&framework).to_lowercase()
            )
        } else {
            "tests written beside the code they check".to_string()
        };

        out.push(Suite {
            id: sid.to_string(),
            path,
            framework_label: say::framework_noun(&framework).to_string(),
            framework,
            declared,
            whole_file,
            covers,
            note,
        });
    }
    out.sort_by(|a, b| b.declared.cmp(&a.declared).then(a.path.cmp(&b.path)));
    Ok(out)
}

fn framework_by_file(loaded: &Loaded) -> Result<BTreeMap<String, String>, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let mut out = BTreeMap::new();
    for (path, sid) in query::present_files(index, store, loaded.prefix())? {
        if let Some(framework) =
            twin::latest(index, store, &sid, "test_framework").map_err(|e| e.to_string())?
        {
            out.insert(path, framework);
        }
    }
    Ok(out)
}

/// Assets grouped by the case that owns them.
fn attachments_by_owner(loaded: &Loaded) -> Result<BTreeMap<StableId, Vec<Attachment>>, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();

    let mut out: BTreeMap<StableId, Vec<Attachment>> = BTreeMap::new();
    for (sid, labels) in query::scoped(index, store, prefix, "asset")? {
        let Some((_, owner)) = twin::live_from(index, store, &sid, "attached_to")
            .map_err(|e| e.to_string())?
            .into_iter()
            .next()
        else {
            continue;
        };
        if query::kind_of(index, store, &owner).as_deref() != Some("test_case") {
            continue;
        }
        let subtype = twin::latest(index, store, &sid, "subtype")
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| assets::infer_subtype(labels.get("path").map(String::as_str).unwrap_or("")).to_string());
        let path = labels.get("path").cloned().unwrap_or_default();
        out.entry(owner).or_default().push(Attachment {
            id: sid.to_string(),
            label: path.rsplit('/').next().unwrap_or(&path).to_string(),
            noun: say::attachment_noun(&subtype).to_string(),
            subtype,
            path,
        });
    }
    for list in out.values_mut() {
        list.sort_by(|a, b| a.label.cmp(&b.label));
    }
    Ok(out)
}
