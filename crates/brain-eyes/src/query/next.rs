//! Next: the work queue — what should happen now, ranked worst-first.
//!
//! The agent asks `brain next`; this is the same queue for the person
//! steering, in the same voice as everything else. One list instead of
//! five reports: failing tests, changes that never settled, documents
//! that may be wrong, features that are not finished, contradictions,
//! open plans. Every row carries the command that acts on it — Eyes
//! never writes.

use crate::dto::{Concern, NextView};
use crate::say;
use crate::state::Loaded;
use brain_observe::agenda;

pub fn build(loaded: &Loaded) -> Result<NextView, String> {
    let prefix = loaded.prefix();
    let items = agenda::queue(&loaded.store, &loaded.index, prefix).map_err(|e| e.to_string())?;

    // (rank, concern): the agent-side scores carry over so both seats see
    // the same order.
    let mut rows: Vec<(u32, Concern)> = Vec::new();
    let mut push = |score: u32, severity: &str, title: String, reason: String, fix: Option<String>| {
        rows.push((
            score,
            Concern {
                severity: severity.to_string(),
                title,
                reason,
                fix_command: fix,
                target: None,
                repeats: 1,
                also: Vec::new(),
            },
        ));
    };

    for item in &items {
        match item.kind.as_str() {
            "failing_test" => push(
                item.score,
                "act",
                format!("{} is failing", item.label),
                "its latest recorded result is a failure".to_string(),
                Some(item.via.clone()),
            ),
            "change" if item.why.contains("reconcile") => push(
                item.score,
                "act",
                format!("the change {} never settled", item.label),
                "it must be reconciled before anything retries it".to_string(),
                Some(item.via.clone()),
            ),
            "change" => push(
                item.score,
                "watch",
                format!("the change {} is waiting for a decision", item.label),
                "proposed, not yet applied".to_string(),
                Some(item.via.clone()),
            ),
            "stale_doc" => push(
                item.score,
                "watch",
                format!("{} may be wrong", item.label),
                item.why.clone(),
                Some(item.via.clone()),
            ),
            "feature_gap" => push(
                item.score,
                "watch",
                format!("{} is not finished", item.label),
                humanize_gap(&item.why),
                Some(item.via.clone()),
            ),
            // Contradictions come from the findings pass below, already
            // in the human voice — the raw rows would repeat them.
            "coherence" => {}
            "plan" => {
                let (slug, title) = item
                    .label
                    .split_once(": ")
                    .unwrap_or((item.label.as_str(), item.label.as_str()));
                push(
                    item.score,
                    "note",
                    format!("the plan \u{201c}{title}\u{201d} is open"),
                    "close it when the work ships, so it stops asking".to_string(),
                    Some(format!("brain plan done {prefix} {slug}")),
                );
            }
            _ => push(
                item.score,
                "note",
                item.label.clone(),
                item.why.clone(),
                Some(item.via.clone()),
            ),
        }
    }

    // Contradictions between things the graph holds, minus the kinds the
    // queue above already carries (a change that never settled would
    // otherwise appear twice).
    for finding in loaded.findings() {
        if matches!(finding.kind.as_str(), "stuck-change" | "broken-change") {
            continue;
        }
        let (title, reason) = say::finding(&finding.kind, &finding.label, &finding.detail);
        let (score, severity, fix) = match finding.kind.as_str() {
            "incoherent-feature" => (
                85,
                "act",
                Some(format!("brain done {prefix} {}", finding.label)),
            ),
            "dangling-test" => (65, "watch", Some(format!("brain twin tests {prefix}"))),
            "uncorroborated-claim" => (50, "note", Some(format!("brain spine {prefix}"))),
            _ => (50, "note", Some(format!("brain tidy . --prefix {prefix}"))),
        };
        rows.push((
            score,
            Concern {
                severity: severity.to_string(),
                title,
                reason,
                fix_command: fix,
                target: None,
                repeats: 1,
                also: Vec::new(),
            },
        ));
    }

    rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.title.cmp(&b.1.title)));
    let queue: Vec<Concern> = rows.into_iter().map(|(_, c)| c).collect();

    let headline = if queue.is_empty() {
        "the queue is empty — nothing failing, nothing waiting".to_string()
    } else {
        format!(
            "{} in the queue, worst first",
            say::count(queue.len() as u64, "thing", "things")
        )
    };
    Ok(NextView {
        snapshot: loaded.snapshot.clone(),
        headline,
        subhead: "act on the top item; the queue re-ranks as the graph learns".to_string(),
        queue,
    })
}

/// `missing tested_by, documented_in` → `not yet tested or documented`.
/// Anything already phrased for people passes through.
fn humanize_gap(why: &str) -> String {
    match why.strip_prefix("missing ") {
        Some(list) => {
            let labels: Vec<&str> = list
                .split(", ")
                .map(crate::say::dod_label)
                .collect();
            format!("not yet {}", labels.join(" or "))
        }
        None => why.to_string(),
    }
}
