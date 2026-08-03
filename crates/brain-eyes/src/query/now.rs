//! "Should I worry, and what changed since I left?"
//!
//! Every concern here is a judgment the graph already computed — coherence
//! findings, staleness with its cause, failing protocols, governed changes
//! that never finished, features claiming more than they can show. None of
//! it was visible in Eyes before. Absence is silence: nothing appears to
//! explain what the graph does not model.

use crate::dto::*;
use crate::query;
use crate::say;
use crate::state::Loaded;
use brain_core::ids::StableId;
use brain_index::Index;
use brain_observe::{attention, sleep, twin};

/// `seen` is the event cursor the viewer's browser remembered from their
/// last visit — per-viewer state the server never stores.
pub fn build(loaded: &Loaded, seen: Option<usize>) -> Result<NowView, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let now = loaded.snapshot.generated_at_ms;

    let insights = loaded.insights();
    let ranked = loaded.attention();
    let findings = loaded.findings();

    let mut needs_you: Vec<Concern> = Vec::new();

    // Failing tests: the loudest thing a codebase can say.
    if !insights.failing.is_empty() {
        let n = insights.failing.len() as u64;
        needs_you.push(Concern {
            severity: "act".to_string(),
            title: format!(
                "{} failing",
                say::count(n, "test is", "tests are")
            ),
            reason: insights
                .failing
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(" · "),
            fix_command: Some(format!("brain twin tests {prefix}")),
            target: None,
            repeats: 1,
            also: Vec::new(),
        });
    }

    // Contradictions between things the graph holds.
    for finding in findings {
        let (title, reason) = say::finding(&finding.kind, &finding.label, &finding.detail);
        let (severity, fix) = match finding.kind.as_str() {
            "stuck-change" | "broken-change" => {
                ("act", Some(format!("brain change show {prefix} {}", finding.label)))
            }
            "incoherent-feature" => ("act", Some(format!("brain done {prefix} {}", finding.label))),
            "dangling-test" => ("watch", Some(format!("brain twin tests {prefix}"))),
            "orphaned-asset" => ("note", Some(format!("brain tidy . --prefix {prefix}"))),
            // Worth knowing about, but not a contradiction — ranking it
            // with one would cry wolf.
            "uncorroborated-claim" => ("note", Some(format!("brain spine {prefix}"))),
            _ => ("watch", None),
        };
        needs_you.push(Concern {
            severity: severity.to_string(),
            title,
            reason,
            fix_command: fix,
            target: None,
            repeats: 1,
            also: Vec::new(),
        });
    }

    // Living documents the code moved out from under.
    for doc in insights
        .stale_docs
        .iter()
        .filter(|d| d.severity == twin::Severity::Warn)
    {
        let sid = StableId::derive(&[doc.kind.as_str(), prefix, doc.slug.as_str()]);
        let changed = doc.changed.join(" · ");
        needs_you.push(Concern {
            severity: "watch".to_string(),
            title: format!(
                "{} may be wrong",
                query::display_name(index, store, &sid, &query::labels_of(index, store, &sid))
            ),
            reason: format!("the code changed after it was written: {changed}"),
            fix_command: Some(format!(
                "brain artifact ack {prefix} {} {}",
                doc.kind, doc.slug
            )),
            target: Some(query::make_ref(index, store, &sid)),
            repeats: 1,
            also: Vec::new(),
        });
    }

    // Governed changes that never reached a conclusion.
    for (sid, labels) in query::scoped(index, store, prefix, "change")? {
        let status = twin::latest(index, store, &sid, "status")
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        if !matches!(status.as_str(), "proposed" | "applied") {
            continue; // broken/indeterminate already arrive as coherence findings
        }
        let slug = labels.get("slug").cloned().unwrap_or_default();
        let (stage, note) = say::change_stage(&status);
        let what = labels.get("title").cloned().unwrap_or_else(|| slug.clone());
        // Name the file it touches. Four tidy moves share a title and are
        // told apart only by what they moved — a collapsed count is worth
        // nothing if unfolding it repeats the same sentence four times.
        let reason = match labels.get("target") {
            Some(path) => format!("{what} — {path}: {note}"),
            None => format!("{what}: {note}"),
        };
        needs_you.push(Concern {
            severity: "note".to_string(),
            title: format!("a governed change is {stage}"),
            reason,
            fix_command: Some(format!("brain change show {prefix} {slug}")),
            target: Some(query::make_ref(index, store, &sid)),
            repeats: 1,
            also: Vec::new(),
        });
    }

    // Records aging quietly: one line, never a list.
    let info_stale = insights
        .stale_docs
        .iter()
        .filter(|d| d.severity == twin::Severity::Info)
        .count();
    if info_stale > 0 {
        needs_you.push(Concern {
            severity: "note".to_string(),
            title: format!(
                "{} written before later changes",
                say::count(info_stale as u64, "record was", "records were")
            ),
            reason: "records are kept as history — they are not expected to track the code"
                .to_string(),
            fix_command: None,
            target: None,
            repeats: 1,
            also: Vec::new(),
        });
    }

    let order = |s: &str| match s {
        "act" => 0,
        "watch" => 1,
        _ => 2,
    };
    needs_you.sort_by_key(|c| order(&c.severity));
    let needs_you = group(needs_you);

    let proof = census(loaded)?;
    let (headline, subhead) = headline(&needs_you, insights, &proof);
    let since = since_last_session(loaded, now)?;
    let attention_cards = attention_cards(loaded, ranked);

    let since_you_looked = seen.map(|seen| {
        say::since_you_looked(loaded.snapshot.cursor.saturating_sub(seen) as u64)
    });
    Ok(NowView {
        snapshot: loaded.snapshot.clone(),
        since_you_looked,
        headline,
        subhead,
        quality: quality_lines(insights),
        needs_you,
        since,
        attention: attention_cards,
        proof,
    })
}

/// The quality strip: every measure judged here — direction, deadband,
/// tone — so the client only draws. Worst first: a worsening line
/// outranks a holding one, which outranks an improving one.
fn quality_lines(insights: &twin::Insights) -> Vec<QualityLine> {
    // Under this many percentage points a ratio's move reads flat — an
    // arrow that twitches on every run would cry wolf.
    const DEADBAND_PP: f64 = 2.0;

    let readings = &insights.quality;
    if readings.is_empty() {
        return Vec::new();
    }

    let mut lines: Vec<(usize, QualityLine)> = Vec::new();

    // Tests: percent passing, counting only readings that had a run.
    let tests: Vec<(usize, usize)> = readings.iter().filter_map(|p| p.tests).collect();
    if let Some(&(passed, total)) = tests.last() {
        let points: Vec<f64> = tests.iter().map(|&(p, t)| ratio(p, t)).collect();
        let prev = (tests.len() >= 2).then(|| tests[tests.len() - 2]);
        let trend = ratio_trend(&points, DEADBAND_PP);
        let (current, sentence) = say::quality_tests(passed, total, prev, trend);
        lines.push((0, line("tests", "Tests passing", points, current, trend, tone(trend, true), sentence)));
    }

    // Features: percent ready, counting only readings that had features.
    let feats: Vec<(usize, usize)> = readings
        .iter()
        .filter(|p| p.features_total > 0)
        .map(|p| (p.features_done, p.features_total))
        .collect();
    if let Some(&(done, total)) = feats.last() {
        let points: Vec<f64> = feats.iter().map(|&(d, t)| ratio(d, t)).collect();
        let prev = (feats.len() >= 2).then(|| feats[feats.len() - 2]);
        let trend = ratio_trend(&points, DEADBAND_PP);
        let (current, sentence) = say::quality_features(done, total, prev, trend);
        lines.push((1, line("features", "Features ready", points, current, trend, tone(trend, true), sentence)));
    }

    // Documents in doubt and claims without proof: plain counts, where
    // any step is a real move and zero is said, not hidden.
    let docs: Vec<f64> = readings.iter().map(|p| p.stale_warnings as f64).collect();
    let n = readings.last().map(|p| p.stale_warnings).unwrap_or(0);
    let prev = (readings.len() >= 2).then(|| readings[readings.len() - 2].stale_warnings);
    let trend = count_trend(&docs);
    let (current, sentence) = say::quality_docs(n, prev, trend);
    lines.push((2, line("docs", "Documents in doubt", docs, current, trend, tone(trend, false), sentence)));

    let claims: Vec<f64> = readings.iter().map(|p| p.uncorroborated as f64).collect();
    let n = readings.last().map(|p| p.uncorroborated).unwrap_or(0);
    let prev = (readings.len() >= 2).then(|| readings[readings.len() - 2].uncorroborated);
    let trend = count_trend(&claims);
    let (current, sentence) = say::quality_claims(n, prev, trend);
    lines.push((3, line("claims", "Claims without proof", claims, current, trend, tone(trend, false), sentence)));

    // Regressions first, then holding, then improving; canonical order
    // within a rank so the strip never reshuffles without cause.
    lines.sort_by_key(|(idx, l)| {
        let rank = match l.tone.as_str() {
            "bad" => 0,
            "quiet" => 1,
            _ => 2,
        };
        (rank, *idx)
    });
    lines.into_iter().map(|(_, l)| l).collect()
}

fn line(
    id: &str,
    label: &str,
    points: Vec<f64>,
    current: String,
    trend: &str,
    tone: &str,
    sentence: String,
) -> QualityLine {
    QualityLine {
        id: id.to_string(),
        label: label.to_string(),
        points,
        current,
        trend: trend.to_string(),
        tone: tone.to_string(),
        sentence,
    }
}

fn ratio(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 * 100.0 / whole as f64
    }
}

/// Direction of a ratio line, with the deadband applied.
pub(crate) fn ratio_trend(points: &[f64], deadband: f64) -> &'static str {
    if points.len() < 2 {
        return "flat";
    }
    let step = points[points.len() - 1] - points[points.len() - 2];
    if step.abs() < deadband {
        "flat"
    } else if step > 0.0 {
        "rising"
    } else {
        "falling"
    }
}

/// Direction of a count line: any step is a real move.
pub(crate) fn count_trend(points: &[f64]) -> &'static str {
    if points.len() < 2 {
        return "flat";
    }
    let step = points[points.len() - 1] - points[points.len() - 2];
    if step > 0.0 {
        "rising"
    } else if step < 0.0 {
        "falling"
    } else {
        "flat"
    }
}

/// Judge a direction: for measures where higher is better, a falling
/// line is the alarm; for debts, a rising one.
fn tone(trend: &str, higher_is_better: bool) -> &'static str {
    match (trend, higher_is_better) {
        ("flat", _) => "quiet",
        ("rising", true) | ("falling", false) => "good",
        _ => "bad",
    }
}

/// Four identical concerns are one concern that happened four times.
///
/// Tidy archiving four plans produced four rows reading "a governed change
/// is applied" — the same sentence, four times, which is noise rather than
/// four things to decide about.
fn group(concerns: Vec<Concern>) -> Vec<Concern> {
    let mut out: Vec<Concern> = Vec::new();
    for concern in concerns {
        if let Some(existing) = out
            .iter_mut()
            .find(|c| c.title == concern.title && c.severity == concern.severity)
        {
            existing.repeats += 1;
            // One example is enough to recognise the kind; the count says
            // how much of it there is.
            if existing.also.len() < 4 {
                existing.also.push(concern.reason.clone());
            }
            continue;
        }
        out.push(Concern {
            repeats: 1,
            also: Vec::new(),
            ..concern
        });
    }
    out
}

/// Every claim the graph makes, reduced to one mark each.
fn census(loaded: &Loaded) -> Result<ProofCensus, String> {
    let evidence = loaded.evidence()?;
    let mut groups: Vec<ProofGroup> = Vec::new();
    for category in &evidence.categories {
        let cells: Vec<ProofCell> = evidence
            .claims
            .iter()
            .filter(|claim| claim.category == category.id)
            .map(|claim| ProofCell {
                id: claim.id.clone(),
                state: if claim.supported { "ready" } else { "unproven" }.to_string(),
                text: format!("{} — {}", claim.claim, claim.verdict),
            })
            .collect();
        groups.push(ProofGroup {
            label: category.label.clone(),
            proven: category.supported,
            total: cells.len(),
            cells,
        });
    }
    let proven = groups.iter().map(|g| g.proven).sum::<usize>();
    let total = groups.iter().map(|g| g.total).sum::<usize>();
    let sentence = if total == 0 {
        "Nothing here claims anything yet.".to_string()
    } else if proven == total {
        format!("Every one of the {total} claims can show its proof.")
    } else {
        format!("{proven} of {total} claims can show their proof.")
    };
    Ok(ProofCensus {
        proven,
        total,
        sentence,
        groups,
    })
}

fn headline(
    concerns: &[Concern],
    insights: &twin::Insights,
    proof: &ProofCensus,
) -> (String, String) {
    let acts = concerns.iter().filter(|c| c.severity == "act").count();
    let watches = concerns.iter().filter(|c| c.severity == "watch").count();
    if let Some(first) = concerns.iter().find(|c| c.severity == "act") {
        let more = acts.saturating_sub(1);
        let subhead = if more > 0 {
            format!("{}, and {more} more need a decision.", first.reason)
        } else {
            first.reason.clone()
        };
        return (capitalize(&first.title), subhead);
    }
    if watches > 0 {
        return (
            format!(
                "{} drifted from the code.",
                say::count(watches as u64, "document has", "documents have")
            ),
            "Nothing is broken — these were written before changes landed underneath them."
                .to_string(),
        );
    }
    let passing = insights
        .last_run
        .map(|(_, total, passed, _)| format!("{passed} of {total} tests passing"))
        .unwrap_or_else(|| "no test run imported yet".to_string());

    // "Everything checks out" while five things sit under Needs you is the
    // kind of cheerful lie this product exists to refuse.
    let notes: usize = concerns.iter().map(|c| c.repeats).sum();
    if notes > 0 {
        return (
            format!(
                "{} worth knowing about.",
                say::count(notes as u64, "thing is", "things are")
            ),
            format!("Nothing is broken: {passing}, and no contradictions."),
        );
    }
    if proof.proven < proof.total {
        return (
            proof.sentence.clone(),
            format!("Nothing is broken: {passing}, and no contradictions."),
        );
    }
    (
        "Everything checks out.".to_string(),
        format!("{passing}; no contradictions and no drifting documents."),
    )
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    let sentence = match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    };
    if sentence.ends_with('.') {
        sentence
    } else {
        format!("{sentence}.")
    }
}

fn since_last_session(loaded: &Loaded, now: u64) -> Result<SinceLastSession, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let repo = StableId::derive(&["repo", prefix]);
    let watermark: u64 = twin::latest(index, store, &repo, "consolidated_until")
        .map_err(|e| e.to_string())?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let delta = sleep::delta_since(store, index, prefix, watermark).map_err(|e| e.to_string())?;
    let episodes = super::timeline::episodes_since(loaded, watermark, 6)?;

    let mut parts = Vec::new();
    if !delta.added.is_empty() {
        parts.push(say::count(delta.added.len() as u64, "new file", "new files"));
    }
    if !delta.changed.is_empty() {
        parts.push(format!(
            "{} changed",
            say::count(delta.changed.len() as u64, "file", "files")
        ));
    }
    if delta.doc_updates > 0 {
        parts.push(format!(
            "{} updated",
            say::count(delta.doc_updates as u64, "document", "documents")
        ));
    }
    if delta.new_runs > 0 {
        parts.push(format!(
            "{} imported",
            say::count(delta.new_runs as u64, "test run", "test runs")
        ));
    }

    let summary = if parts.is_empty() {
        "Nothing has changed since then.".to_string()
    } else {
        capitalize(&parts.join(", "))
    };

    Ok(SinceLastSession {
        known: watermark > 0,
        when: (watermark > 0).then(|| say::ago(now, watermark)),
        summary,
        episodes,
    })
}

fn attention_cards(loaded: &Loaded, ranked: &[attention::Attention]) -> Vec<AttentionCard> {
    let index = &loaded.index;
    ranked
        .iter()
        .filter_map(|item| {
            let reasons: Vec<String> = item
                .reasons
                .iter()
                .filter_map(|raw| say::attention_reason(raw))
                .collect();
            // A card with nothing to say is noise.
            if reasons.is_empty() {
                return None;
            }
            let sid = StableId::derive(&["file", &item.label]);
            let known = !index.entity_nodes(&sid).is_empty();
            Some(AttentionCard {
                label: item.label.clone(),
                kind: item.kind.clone(),
                noun: say::kind_noun(&item.kind).to_string(),
                glyph: say::kind_glyph(&item.kind).to_string(),
                id: known.then(|| sid.to_string()),
                reasons,
            })
        })
        .take(6)
        .collect()
}

