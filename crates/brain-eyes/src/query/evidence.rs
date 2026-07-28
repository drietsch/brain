//! "What is claimed, and what stands behind it?"
//!
//! The rule from the briefing: a claim must never be visually stronger
//! than its proof. That is easy to say and hard to mean, because the
//! honest answer here is often *less* than it looks. A feature's
//! definition of done counts linked records; it does **not** check that a
//! linked test passed. So "tested — 3 records linked" is a claim, and the
//! current result of those tests is the proof, and the two can disagree.
//!
//! Making that disagreement visible is the whole point of this surface.
//! Every claim resolves its own proof at read time and says plainly when
//! the proof does not hold it up.

use crate::dto::*;
use crate::query;
use crate::say;
use crate::state::Loaded;
use brain_core::object::Object;
use brain_index::Index;
use brain_observe::{features, twin};

pub fn build(loaded: &Loaded) -> Result<EvidenceView, String> {
    let mut claims = Vec::new();
    claims.extend(feature_claims(loaded)?);
    claims.extend(run_claims(loaded)?);
    claims.extend(change_claims(loaded)?);
    claims.extend(projection_claims(loaded)?);
    claims.extend(document_claims(loaded)?);

    // Unsupported first: an evidence browser that opens on things that are
    // fine is answering a question nobody asked.
    claims.sort_by(|a, b| {
        a.supported
            .cmp(&b.supported)
            .then(a.category.cmp(&b.category))
            .then(a.claim.cmp(&b.claim))
    });

    let categories = categories(&claims);
    let unsupported = claims.iter().filter(|c| !c.supported).count();
    let headline = if claims.is_empty() {
        "Nothing here claims anything yet.".to_string()
    } else if unsupported == 0 {
        format!(
            "Every one of the {} claims on this page can show its proof.",
            claims.len()
        )
    } else {
        format!(
            "{} of {} claims cannot show proof.",
            unsupported,
            claims.len()
        )
    };

    Ok(EvidenceView {
        snapshot: loaded.snapshot.clone(),
        headline,
        categories,
        claims,
    })
}

const CATEGORY_NOTES: &[(&str, &str, &str)] = &[
    (
        "features",
        "Feature completeness",
        "What each feature claims, and whether the records behind it agree.",
    ),
    (
        "tests",
        "Test evidence",
        "Runs that were observed, and what they established.",
    ),
    (
        "changes",
        "Action verification",
        "Edits the brain made, and whether anything checked them.",
    ),
    (
        "projections",
        "Artifact verification",
        "Generated files, and whether their bytes still match what produced them.",
    ),
    (
        "documents",
        "Documentation freshness",
        "Prose that claims to describe code that has since moved.",
    ),
];

fn categories(claims: &[Claim]) -> Vec<EvidenceCategory> {
    CATEGORY_NOTES
        .iter()
        .filter_map(|(id, label, note)| {
            let mine: Vec<&Claim> = claims.iter().filter(|c| c.category == *id).collect();
            if mine.is_empty() {
                return None; // absence is silence
            }
            Some(EvidenceCategory {
                id: id.to_string(),
                label: label.to_string(),
                note: note.to_string(),
                supported: mine.iter().filter(|c| c.supported).count(),
                unsupported: mine.iter().filter(|c| !c.supported).count(),
            })
        })
        .collect()
}

/// A feature's definition of done, with each slot's evidence resolved to
/// its *current* state rather than merely counted.
fn feature_claims(loaded: &Loaded) -> Result<Vec<Claim>, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();

    let mut out = Vec::new();
    for (sid, labels) in query::scoped(index, store, prefix, "feature")? {
        let slug = labels.get("slug").cloned().unwrap_or_default();
        let title = query::display_name(index, store, &sid, &labels);
        let report = features::evaluate(store, index, prefix, &slug).map_err(|e| e.to_string())?;

        let mut proof = Vec::new();
        let mut holds = report.done;

        // A feature with parts is judged by its parts, so its parts are
        // its proof. Each one reports its own readiness — never merely
        // that it exists.
        for part in &report.parts {
            let part_sid = features::feature_sid(prefix, &part.slug);
            proof.push(Proof {
                text: if part.done {
                    format!("{} is ready ({}/{})", part.title, part.met, part.total)
                } else {
                    format!(
                        "{} is not ready yet ({}/{})",
                        part.title, part.met, part.total
                    )
                },
                basis: Some(if part.met == 0 {
                    "nothing is linked to it at all".to_string()
                } else {
                    "judged the same way, against its own requirements".to_string()
                }),
                tone: if part.done { "good" } else { "watch" }.to_string(),
                target: Some(query::make_ref(index, store, &part_sid)),
            });
        }

        for check in &report.checks {
            let label = say::dod_label(&check.predicate);
            if check.count == 0 {
                // A parent judged by its parts is not failing because it
                // has no direct links of its own; that is the normal shape.
                if report.by_parts() {
                    continue;
                }
                proof.push(Proof {
                    text: format!("nothing is linked as {label}"),
                    basis: None,
                    tone: "bad".to_string(),
                    target: None,
                });
                continue;
            }
            // Resolve each linked record to what it currently says. No
            // silent truncation: a claim that hides half its evidence is
            // the failure this surface exists to prevent.
            let linked = twin::live_from(index, store, &sid, &check.predicate)
                .map_err(|e| e.to_string())?;
            for (_, to) in &linked {
                let reference = query::make_ref(index, store, to);
                let (text, basis, tone) = resolve_link(loaded, to, label, &reference)?;
                // A direct link on a parent is supporting detail, not the
                // verdict — the parts decide.
                if tone == "bad" && !report.by_parts() {
                    holds = false;
                }
                proof.push(Proof {
                    text,
                    basis,
                    tone,
                    target: Some(reference),
                });
            }
        }

        let (met, total) = report.score();
        let verdict = if report.by_parts() {
            match (&report.blocked_by, report.done) {
                (_, true) => format!("every one of its {total} parts is ready"),
                (Some(blocking), _) => {
                    format!("{met} of {total} parts are ready; waiting on {blocking}")
                }
                (None, _) => format!("{met} of {total} parts are ready"),
            }
        } else if report.done && holds {
            "every requirement is linked, and every linked record still stands".to_string()
        } else if report.done {
            "every requirement is linked, but a linked record no longer holds".to_string()
        } else {
            format!("{met} of {total} requirements are linked")
        };

        out.push(Claim {
            id: sid.to_string(),
            subject: Some(query::make_ref(index, store, &sid)),
            claim: format!("{title} is complete"),
            supported: report.done && holds,
            tone: if report.done && holds { "good" } else { "watch" }.to_string(),
            verdict,
            proof,
            category: "features".to_string(),
            fix_command: Some(format!("brain done {prefix} {slug}")),
        });
    }
    Ok(out)
}

/// What a linked definition-of-done target currently says about itself.
pub(crate) fn resolve_link(
    loaded: &Loaded,
    target: &brain_core::ids::StableId,
    label: &str,
    reference: &Ref,
) -> Result<(String, Option<String>, String), String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let name = &reference.label;

    match reference.kind.as_str() {
        "test_case" => {
            let raw = twin::latest(index, store, target, "result")
                .map_err(|e| e.to_string())?
                .unwrap_or_default();
            let (result, tone) = say::test_result(&raw);
            Ok((
                format!("{name} is {result}"),
                Some(say::evidence_level("behavioral").to_string()),
                tone.to_string(),
            ))
        }
        "source_file" => {
            // A file linked as `tested_by` is a claim about coverage; check it.
            if label == "tested" {
                let declared = twin::latest(index, store, target, "tests_declared")
                    .map_err(|e| e.to_string())?
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(0);
                if declared == 0 {
                    return Ok((
                        format!("{name} is linked as the test, but declares no tests"),
                        None,
                        "bad".to_string(),
                    ));
                }
                return Ok((
                    format!("{name} declares {}", say::count(declared as u64, "test", "tests")),
                    Some("its shape was checked, not its result".to_string()),
                    "watch".to_string(),
                ));
            }
            Ok((format!("{name} is linked as {label}"), None, "good".to_string()))
        }
        // A linked feature must answer for itself. Reporting it as good
        // merely because it exists would let a parent look supported
        // while the thing supporting it was nowhere near done.
        "feature" => {
            let slug = query::labels_of(index, store, target)
                .get("slug")
                .cloned()
                .unwrap_or_default();
            let report = features::evaluate(store, index, loaded.prefix(), &slug)
                .map_err(|e| e.to_string())?;
            let (met, total) = report.score();
            let terms = if report.by_parts() { "parts" } else { "requirements" };
            Ok(if report.done {
                (
                    format!("{name} is ready ({met}/{total} {terms})"),
                    Some("judged against its own requirements".to_string()),
                    "good".to_string(),
                )
            } else {
                (
                    format!("{name} is not ready yet ({met}/{total} {terms})"),
                    report
                        .blocked_by
                        .map(|blocking| format!("waiting on {blocking}")),
                    "bad".to_string(),
                )
            })
        }
        _ => {
            let (state, why) = brain_observe::lifecycle::of(index, store, target)
                .map_err(|e| e.to_string())?;
            if !state.is_active() {
                let sentence = say::lifecycle(state.as_str(), &why)
                    .unwrap_or_else(|| state.as_str().to_string());
                return Ok((
                    format!("{name} is {sentence}"),
                    None,
                    "watch".to_string(),
                ));
            }
            Ok((format!("{name} is linked as {label}"), None, "good".to_string()))
        }
    }
}

/// The Evidence objects test runs actually wrote.
fn run_claims(loaded: &Loaded) -> Result<Vec<Claim>, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let now = loaded.snapshot.generated_at_ms;

    let repo = brain_core::ids::StableId::derive(&["repo", prefix]);
    let mut out = Vec::new();
    for node in index.entity_nodes(&repo) {
        for evidence_node in index.evidence_for(&node) {
            let Ok(Object::Evidence {
                level,
                method,
                passed,
                detail,
                ..
            }) = store.get(&evidence_node)
            else {
                continue;
            };
            let level = format!("{level:?}").to_lowercase();
            out.push(Claim {
                id: evidence_node.to_string(),
                subject: None,
                claim: if passed {
                    "the suite passed".to_string()
                } else {
                    "the suite failed".to_string()
                },
                supported: passed,
                tone: if passed { "good" } else { "bad" }.to_string(),
                verdict: detail.clone(),
                proof: vec![Proof {
                    text: format!("recorded when the run was imported ({})", short_method(&method)),
                    basis: Some(say::evidence_level(&level).to_string()),
                    tone: if passed { "good" } else { "bad" }.to_string(),
                    target: None,
                }],
                category: "tests".to_string(),
                fix_command: None,
            });
        }
    }
    // Newest evidence is the only evidence anyone reads; the graph keeps
    // the rest and this surface shows a handful.
    out.reverse();
    out.truncate(6);
    let _ = now;
    Ok(out)
}

fn short_method(method: &str) -> String {
    method
        .split_once('@')
        .map(|(kind, _)| format!("{kind} protocol"))
        .unwrap_or_else(|| method.to_string())
}

/// Governed changes: applied is not verified.
fn change_claims(loaded: &Loaded) -> Result<Vec<Claim>, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();

    let mut out = Vec::new();
    for (sid, labels) in query::scoped(index, store, prefix, "change")? {
        let status = twin::latest(index, store, &sid, "status")
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        if status == "proposed" {
            continue; // nothing has happened yet, so nothing is claimed
        }
        let slug = labels.get("slug").cloned().unwrap_or_default();
        let target = labels.get("target").cloned().unwrap_or_else(|| slug.clone());
        let (stage, note) = say::change_stage(&status);

        let mut proof = Vec::new();
        for (_, run) in twin::live_from(index, store, &sid, "verified_by")
            .map_err(|e| e.to_string())?
        {
            let failed = twin::latest(index, store, &run, "failed")
                .map_err(|e| e.to_string())?
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0);
            let total = twin::latest(index, store, &run, "total")
                .map_err(|e| e.to_string())?
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0);
            proof.push(Proof {
                text: format!("a run of {total} tests, {failed} failing, followed this change"),
                basis: Some(say::evidence_level("behavioral").to_string()),
                tone: if failed == 0 { "good" } else { "bad" }.to_string(),
                target: Some(query::make_ref(index, store, &run)),
            });
        }
        if proof.is_empty() {
            proof.push(Proof {
                text: "no test run is linked to this change".to_string(),
                basis: None,
                tone: "bad".to_string(),
                target: None,
            });
        }

        let supported = status == "verified";
        out.push(Claim {
            id: sid.to_string(),
            subject: Some(query::make_ref(index, store, &sid)),
            claim: format!("the edit to {target} is {stage}"),
            supported,
            tone: if supported { "good" } else { "watch" }.to_string(),
            verdict: note.to_string(),
            proof,
            category: "changes".to_string(),
            fix_command: (!supported).then(|| format!("brain change verify {prefix} {slug}")),
        });
    }
    Ok(out)
}

/// Generated files whose bytes no longer match what produced them.
fn projection_claims(loaded: &Loaded) -> Result<Vec<Claim>, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();

    let mut out = Vec::new();
    for (path, sid) in query::present_files(index, store, prefix)? {
        let Some(expected) =
            twin::latest(index, store, &sid, "expected_b3").map_err(|e| e.to_string())?
        else {
            continue;
        };
        let actual = twin::latest(index, store, &sid, "content_b3")
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let matches = actual == expected;
        let rendered_from = twin::latest(index, store, &sid, "rendered_from")
            .map_err(|e| e.to_string())?;

        let mut proof = vec![Proof {
            text: if matches {
                "its bytes are exactly what the last render produced".to_string()
            } else {
                "its bytes differ from what the last render produced — someone edited it by hand"
                    .to_string()
            },
            basis: Some("its shape was checked".to_string()),
            tone: if matches { "good" } else { "bad" }.to_string(),
            target: None,
        }];
        if let Some(command) = &rendered_from {
            proof.push(Proof {
                text: format!("produced by `{command}`"),
                basis: None,
                tone: "quiet".to_string(),
                target: None,
            });
        }

        out.push(Claim {
            id: sid.to_string(),
            subject: Some(query::make_ref(index, store, &sid)),
            claim: format!("{path} is generated, not written"),
            supported: matches,
            tone: if matches { "good" } else { "bad" }.to_string(),
            verdict: if matches {
                "unchanged since it was rendered".to_string()
            } else {
                "hand-edited since it was rendered; regenerating will discard the edit".to_string()
            },
            proof,
            category: "projections".to_string(),
            fix_command: (!matches).then(|| {
                rendered_from
                    .clone()
                    .unwrap_or_else(|| format!("brain artifact render . --prefix {prefix} --check"))
            }),
        });
    }
    // A clean repository has dozens of these; show the broken ones and a
    // sample of the rest.
    let (mut broken, intact): (Vec<Claim>, Vec<Claim>) =
        out.into_iter().partition(|claim| !claim.supported);
    broken.extend(intact.into_iter().take(6));
    Ok(broken)
}

/// Documents that describe code which has since moved.
fn document_claims(loaded: &Loaded) -> Result<Vec<Claim>, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let insights = loaded.insights();

    let mut out = Vec::new();
    for doc in &insights.stale_docs {
        if doc.severity != twin::Severity::Warn {
            continue; // info-level ageing is normal for a record
        }
        let sid = brain_core::ids::StableId::derive(&[&doc.kind, loaded.prefix(), &doc.slug]);
        let reference = query::make_ref(index, store, &sid);
        out.push(Claim {
            id: sid.to_string(),
            claim: format!("{} describes the code as it is", reference.label),
            supported: false,
            tone: "watch".to_string(),
            verdict: "the code changed after this was written".to_string(),
            proof: doc
                .changed
                .iter()
                .take(6)
                .map(|changed| Proof {
                    text: format!("{changed} changed since"),
                    basis: None,
                    tone: "bad".to_string(),
                    target: None,
                })
                .collect(),
            category: "documents".to_string(),
            fix_command: Some(format!(
                "brain artifact ack {} {} {}",
                loaded.prefix(),
                doc.kind,
                doc.slug
            )),
            subject: Some(reference),
        });
    }
    Ok(out)
}
