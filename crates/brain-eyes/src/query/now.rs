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
use brain_observe::{attention, features, lifecycle, sleep, twin};

pub fn build(loaded: &Loaded) -> Result<NowView, String> {
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
            _ => ("watch", None),
        };
        needs_you.push(Concern {
            severity: severity.to_string(),
            title,
            reason,
            fix_command: fix,
            target: None,
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
        needs_you.push(Concern {
            severity: "note".to_string(),
            title: format!("a governed change is {stage}"),
            reason: format!("{}: {note}", labels.get("title").cloned().unwrap_or(slug.clone())),
            fix_command: Some(format!("brain change show {prefix} {slug}")),
            target: Some(query::make_ref(index, store, &sid)),
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
        });
    }

    let order = |s: &str| match s {
        "act" => 0,
        "watch" => 1,
        _ => 2,
    };
    needs_you.sort_by_key(|c| order(&c.severity));

    let (headline, subhead) = headline(&needs_you, insights);
    let since = since_last_session(loaded, now)?;
    let attention_cards = attention_cards(loaded, ranked);
    let stats = stats(loaded, insights)?;

    Ok(NowView {
        snapshot: loaded.snapshot.clone(),
        headline,
        subhead,
        needs_you,
        since,
        attention: attention_cards,
        stats,
    })
}

fn headline(concerns: &[Concern], insights: &twin::Insights) -> (String, String) {
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
        .map(|(_, total, passed, _)| format!("{passed}/{total} tests passing"))
        .unwrap_or_else(|| "no test run imported yet".to_string());
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

fn stats(loaded: &Loaded, insights: &twin::Insights) -> Result<Vec<Stat>, String> {
    let store = &loaded.store;
    let index: &brain_index::MemIndex = &loaded.index;
    let prefix = loaded.prefix();
    let mut out = Vec::new();

    if let Some((at, total, passed, failed)) = insights.last_run {
        out.push(Stat {
            label: "Tests".to_string(),
            value: format!("{passed} of {total} passing"),
            note: Some(format!("last run {}", say::ago(loaded.snapshot.generated_at_ms, at))),
            tone: if failed == 0 { "good" } else { "bad" }.to_string(),
        });
    } else {
        out.push(Stat {
            label: "Tests".to_string(),
            value: "no run imported yet".to_string(),
            note: Some("nothing has told the graph how the tests went".to_string()),
            tone: "quiet".to_string(),
        });
    }

    // Features that can show everything their contract asks for.
    let mut complete = 0usize;
    let rows = features::list(store, index, prefix).map_err(|e| e.to_string())?;
    for row in &rows {
        let report =
            features::evaluate(store, index, prefix, &row.slug).map_err(|e| e.to_string())?;
        if report.done {
            complete += 1;
        }
    }
    if !rows.is_empty() {
        out.push(Stat {
            label: "Features".to_string(),
            value: format!("{complete} of {} complete", rows.len()),
            note: Some("built, tested, decided and documented".to_string()),
            tone: if complete == rows.len() { "good" } else { "watch" }.to_string(),
        });
    }

    let living = living_documents(loaded)?;
    let warn = insights
        .stale_docs
        .iter()
        .filter(|d| d.severity == twin::Severity::Warn)
        .count();
    if living > 0 {
        out.push(Stat {
            label: "Documents".to_string(),
            value: format!("{} of {living} current", living.saturating_sub(warn)),
            note: Some("documents expected to track the code".to_string()),
            tone: if warn == 0 { "good" } else { "watch" }.to_string(),
        });
    }

    out.push(Stat {
        label: "Code".to_string(),
        value: format!(
            "{} files, {} functions and types",
            insights.files, insights.symbols
        ),
        note: None,
        tone: "quiet".to_string(),
    });

    if let (Some(branch), Some(commit)) = (&insights.git_branch, &insights.git_commit) {
        out.push(Stat {
            label: "Git".to_string(),
            value: branch.clone(),
            note: Some(commit.chars().take(12).collect()),
            tone: "quiet".to_string(),
        });
    }
    Ok(out)
}

/// Documents whose kind is expected to track the code (rot policy `warn`),
/// and that are still active.
fn living_documents(loaded: &Loaded) -> Result<usize, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let mut count = 0usize;
    for (kind, def) in loaded.registry() {
        let severity = twin::rot_severity(&def.rot, kind);
        if severity != Some(twin::Severity::Warn) {
            continue;
        }
        for (sid, _) in query::scoped(index, store, prefix, kind)? {
            let (state, _) = lifecycle::of(index, store, &sid).map_err(|e| e.to_string())?;
            if state.is_active() {
                count += 1;
            }
        }
    }
    Ok(count)
}
