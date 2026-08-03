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
            chips: Vec::new(),
            steps: Vec::new(),
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
            chips: Vec::new(),
            steps: Vec::new(),
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
            chips: Vec::new(),
            steps: Vec::new(),
        });
    }

    // Governed changes that never reached a conclusion: one card per
    // stuck stage, showing the journey (proposed → applied → verified)
    // and the files themselves as openable chips — the card shows what
    // it is about instead of asking the reader to imagine it.
    let mut limbo: std::collections::BTreeMap<String, Vec<(StableId, String, String, u64)>> =
        std::collections::BTreeMap::new();
    for (sid, labels) in query::scoped(index, store, prefix, "change")? {
        let (status_at, status) = twin::latest_at(index, store, &sid, "status")
            .map_err(|e| e.to_string())?
            .unwrap_or((0, String::new()));
        if !matches!(status.as_str(), "proposed" | "applied") {
            continue; // broken/indeterminate already arrive as coherence findings
        }
        let slug = labels.get("slug").cloned().unwrap_or_default();
        let target = labels.get("target").cloned().unwrap_or_else(|| slug.clone());
        limbo.entry(status).or_default().push((sid, slug, target, status_at));
    }
    for (status, mut changes) in limbo {
        changes.sort_by(|a, b| a.3.cmp(&b.3).then(a.2.cmp(&b.2)));
        let oldest = say::ago(now, changes.first().map(|c| c.3).unwrap_or(0));
        let (title, reason) = say::changes_in_limbo(&status, changes.len(), &oldest);
        let step = |label: &str, state: &str, when: Option<String>| Stage {
            label: label.to_string(),
            note: String::new(),
            state: state.to_string(),
            when,
        };
        let steps = if status == "proposed" {
            vec![
                step("proposed", "done", Some(oldest.clone())),
                step("applied", "missing", None),
                step("verified", "missing", None),
            ]
        } else {
            vec![
                step("proposed", "done", None),
                step("applied", "done", Some(oldest.clone())),
                step("verified", "missing", None),
            ]
        };
        // Chips wear the file the change touches — the slug is the
        // machine's name for it, the path is the person's.
        let chips: Vec<Ref> = changes
            .iter()
            .map(|(sid, _, target, _)| {
                let mut reference = query::make_ref(index, store, sid);
                reference.label = target.clone();
                reference
            })
            .collect();
        let first_slug = changes.first().map(|c| c.1.clone()).unwrap_or_default();
        let fix = match status.as_str() {
            "proposed" => format!("brain change apply {prefix} {first_slug} --cap fs"),
            _ => format!("brain change verify {prefix} {first_slug}"),
        };
        needs_you.push(Concern {
            severity: "note".to_string(),
            title,
            reason,
            fix_command: Some(fix),
            target: None,
            repeats: 1,
            also: Vec::new(),
            chips,
            steps,
        });
    }

    // Records aging quietly: one card wearing the records themselves —
    // a shelf of openable chips instead of a bare count.
    let aged: Vec<&twin::StaleDoc> = insights
        .stale_docs
        .iter()
        .filter(|d| d.severity == twin::Severity::Info)
        .collect();
    if !aged.is_empty() {
        let chips: Vec<Ref> = aged
            .iter()
            .map(|doc| {
                let sid = StableId::derive(&[doc.kind.as_str(), prefix, doc.slug.as_str()]);
                query::make_ref(index, store, &sid)
            })
            .collect();
        let (title, reason) = say::records_aged(aged.len());
        needs_you.push(Concern {
            severity: "note".to_string(),
            title,
            reason,
            fix_command: Some(format!("brain twin stale {prefix}")),
            target: None,
            repeats: 1,
            also: Vec::new(),
            chips,
            steps: Vec::new(),
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
    let (headline, subhead) = headline(&needs_you, insights, &proof, now);
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
        quality: quality_lines(insights, now),
        needs_you,
        since,
        attention: attention_cards,
        proof,
    })
}

/// The quality strip: every measure judged here — direction, deadband,
/// tone — so the client only draws. Worst first: a worsening line
/// outranks a holding one, which outranks an improving one.
fn quality_lines(insights: &twin::Insights, now: u64) -> Vec<QualityLine> {
    // Under this many percentage points a ratio's move reads flat — an
    // arrow that twitches on every run would cry wolf.
    const DEADBAND_PP: f64 = 2.0;

    let readings = &insights.quality;
    if readings.is_empty() {
        return Vec::new();
    }

    let mut lines: Vec<(usize, QualityLine)> = Vec::new();

    // Tests: percent passing, counting only readings that had a run.
    // The run's age rides along — a level without its moment can read
    // calm long after anyone last ran anything.
    let tests: Vec<(u64, (usize, usize))> = readings
        .iter()
        .filter_map(|p| p.tests.map(|t| (p.at_ms, t)))
        .collect();
    if let Some(&(_, (passed, total))) = tests.last() {
        let points: Vec<f64> = tests.iter().map(|&(_, (p, t))| ratio(p, t)).collect();
        let at_ms: Vec<u64> = tests.iter().map(|&(at, _)| at).collect();
        let prev = (tests.len() >= 2).then(|| tests[tests.len() - 2].1);
        let trend = ratio_trend(&points, DEADBAND_PP);
        let ran = insights.last_run.map(|(at, ..)| say::ago(now, at));
        let (current, sentence) =
            say::quality_tests(passed, total, prev, trend, ran.as_deref());
        lines.push((0, line("tests", "Tests passing", points, at_ms, current, trend, tone(trend, true), sentence)));
    }

    // Features: percent ready, counting only readings that had features.
    let feats: Vec<(u64, (usize, usize))> = readings
        .iter()
        .filter(|p| p.features_total > 0)
        .map(|p| (p.at_ms, (p.features_done, p.features_total)))
        .collect();
    if let Some(&(_, (done, total))) = feats.last() {
        let points: Vec<f64> = feats.iter().map(|&(_, (d, t))| ratio(d, t)).collect();
        let at_ms: Vec<u64> = feats.iter().map(|&(at, _)| at).collect();
        let prev = (feats.len() >= 2).then(|| feats[feats.len() - 2].1);
        let trend = ratio_trend(&points, DEADBAND_PP);
        let (current, sentence) = say::quality_features(done, total, prev, trend);
        lines.push((1, line("features", "Features ready", points, at_ms, current, trend, tone(trend, true), sentence)));
    }

    // Documents in doubt and claims without proof: plain counts, where
    // any step is a real move and zero is said, not hidden.
    let all_at: Vec<u64> = readings.iter().map(|p| p.at_ms).collect();
    let docs: Vec<f64> = readings.iter().map(|p| p.stale_warnings as f64).collect();
    let n = readings.last().map(|p| p.stale_warnings).unwrap_or(0);
    let prev = (readings.len() >= 2).then(|| readings[readings.len() - 2].stale_warnings);
    let trend = count_trend(&docs);
    let (current, sentence) = say::quality_docs(n, prev, trend);
    lines.push((2, line("docs", "Documents in doubt", docs, all_at.clone(), current, trend, tone(trend, false), sentence)));

    // "Feature claims", not "claims": the census above this strip counts
    // every claim the graph makes, this line counts only what features
    // declare and nothing corroborates — one word for both bred a
    // contradiction three centimetres apart.
    let claims: Vec<f64> = readings.iter().map(|p| p.uncorroborated as f64).collect();
    let n = readings.last().map(|p| p.uncorroborated).unwrap_or(0);
    let prev = (readings.len() >= 2).then(|| readings[readings.len() - 2].uncorroborated);
    let trend = count_trend(&claims);
    let (current, sentence) = say::quality_claims(n, prev, trend);
    lines.push((3, line("claims", "Feature claims", claims, all_at, current, trend, tone(trend, false), sentence)));

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

#[allow(clippy::too_many_arguments)]
fn line(
    id: &str,
    label: &str,
    points: Vec<f64>,
    at_ms: Vec<u64>,
    current: String,
    trend: &str,
    tone: &str,
    sentence: String,
) -> QualityLine {
    QualityLine {
        id: id.to_string(),
        label: label.to_string(),
        points,
        at_ms,
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

pub(crate) fn headline(
    concerns: &[Concern],
    insights: &twin::Insights,
    proof: &ProofCensus,
    now: u64,
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
    // A test count without its age can stay calm long after the tests
    // stopped being run — the run's moment is part of the truth.
    let passing = insights
        .last_run
        .map(|(at, total, passed, _)| {
            format!("{passed} of {total} tests passed {}", say::ago(now, at))
        })
        .unwrap_or_else(|| "no test run imported yet".to_string());

    // Only notes remain: the calm sentence has earned the headline, and
    // the notes are the footnote — but still counted, because "everything
    // checks out" while things sit under Needs you would be a cheerful
    // lie, and an uncounted footnote is how notes rot unread.
    let notes: usize = concerns.iter().map(|c| c.repeats).sum();
    if notes > 0 {
        return (
            "Nothing is broken.".to_string(),
            format!(
                "{passing}, no contradictions — {} below for when you have a minute.",
                say::count(notes as u64, "note", "notes")
            ),
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
            // A card with nothing to say is noise — and a card whose
            // only claim is "widely imported" is not pressure, it is
            // centrality. Pressure needs change or a missing test; the
            // quiet hubs keep their rank in `brain attend` and the Map.
            if reasons.is_empty() || reasons.iter().all(|r| r.contains("import this")) {
                return None;
            }
            let sid = StableId::derive(&["file", &item.label]);
            let known = !index.entity_nodes(&sid).is_empty();
            // The numbers behind the sentences, for the mini-table: the
            // ranking already measured churn, reach and coverage — hand
            // them over as records instead of asking the client to read
            // them back out of prose.
            let mut churn = None;
            let mut reach = None;
            let mut tested = None;
            for raw in &item.reasons {
                if let Some(rest) = raw.strip_prefix("churn ") {
                    churn = rest.split_whitespace().next().and_then(|v| v.parse().ok());
                } else if let Some(rest) = raw.strip_prefix("hub ") {
                    reach = rest.trim().parse().ok();
                } else if raw == "untested hub" {
                    tested = Some(false);
                } else if raw.contains("failing test") {
                    tested = Some(true);
                }
            }
            Some(AttentionCard {
                label: item.label.clone(),
                kind: item.kind.clone(),
                noun: say::kind_noun(&item.kind).to_string(),
                glyph: say::kind_glyph(&item.kind).to_string(),
                id: known.then(|| sid.to_string()),
                reasons,
                churn,
                reach,
                tested,
            })
        })
        .take(6)
        .collect()
}

