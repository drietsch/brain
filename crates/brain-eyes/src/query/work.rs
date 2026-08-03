//! "What is being worked on?" — with, for the first time, an answer that
//! names who.
//!
//! Until agent sessions were ingested this surface could only have shown
//! governed changes and open plans, which in a healthy repository is
//! usually nothing at all. A session is the graph's only record of a
//! principal: what it was asked to do, which model, how long it ran, and
//! which files it changed.
//!
//! Proposed governed changes are the one approval queue the graph does
//! model, so they get a desk: the recorded diff, the pre-apply briefing
//! of the target, and the command that applies it. Eyes still never
//! writes — the decision renders here, the CLI executes it, and the
//! audit trail never forks.

use crate::dto::*;
use crate::query;
use crate::say;
use crate::state::Loaded;
use brain_observe::{lifecycle, sessions, twin};

/// A session whose last recorded activity is inside this window is still
/// considered to be running.
const LIVE_WITHIN_MS: u64 = 20 * 60 * 1000;

/// A live session older than this that has touched nothing may be stuck.
const STUCK_AFTER_MS: u64 = 30 * 60 * 1000;

pub fn build(loaded: &Loaded) -> Result<WorkView, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let now = loaded.snapshot.generated_at_ms;

    let rows = sessions::list(store, index, prefix).map_err(|e| e.to_string())?;
    let mut sessions_out = Vec::new();
    let mut editors: std::collections::BTreeMap<
        brain_core::ids::StableId,
        std::collections::BTreeSet<brain_core::ids::StableId>,
    > = std::collections::BTreeMap::new();
    // The control room's two derived warnings, gathered as the sessions
    // stream past: who is converging on the same file right now, and who
    // has run long with nothing to show. Both are only as fresh as the
    // last import — the sentence says so.
    let mut live_files: std::collections::BTreeMap<brain_core::ids::StableId, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut maybe_stuck: Vec<(String, String)> = Vec::new();
    for row in rows.iter().take(25) {
        let live = now.saturating_sub(row.ended_at_ms) < LIVE_WITHIN_MS;
        let touched_all = twin::live_from(index, store, &row.sid, "touched")
            .map_err(|e| e.to_string())?;
        for (_, file) in &touched_all {
            editors.entry(file.clone()).or_default().insert(row.sid.clone());
        }
        if live {
            for (_, file) in &touched_all {
                live_files
                    .entry(file.clone())
                    .or_default()
                    .push(say::agent_noun(&row.agent).to_string());
            }
            if touched_all.is_empty()
                && now.saturating_sub(row.started_at_ms) > STUCK_AFTER_MS
            {
                maybe_stuck.push((
                    say::agent_noun(&row.agent).to_string(),
                    say::span(row.started_at_ms, row.ended_at_ms),
                ));
            }
        }
        let touched: Vec<Ref> = touched_all
            .iter()
            .take(8)
            .map(|(_, to)| query::make_ref(index, store, to))
            .collect();

        // What it produced is not stored twice: an artifact whose file the
        // session edited is an artifact the session produced.
        let mut produced: Vec<Ref> = Vec::new();
        for (_, file) in &touched_all {
            for (_, from) in twin::live_to(index, store, file, "recorded_in")
                .map_err(|e| e.to_string())?
            {
                let reference = query::make_ref(index, store, &from);
                if reference.kind == "asset" || produced.iter().any(|r| r.id == reference.id) {
                    continue;
                }
                produced.push(reference);
            }
        }
        produced.truncate(6);

        // Several tool names mean the same act — Edit and Write both edit
        // files — so the chips are totalled by what the tool did, not by
        // what it is called. Otherwise a session reports "edited files"
        // three times with three different numbers.
        let mut totals: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for part in row.tools.split(", ").filter(|part| !part.is_empty()) {
            let Some((name, count)) = part.rsplit_once(' ') else {
                continue;
            };
            let Ok(count) = count.parse::<usize>() else {
                continue;
            };
            *totals.entry(tool_label(name)).or_insert(0) += count;
        }
        let mut tools: Vec<ToolUse> = totals
            .into_iter()
            .map(|(label, count)| ToolUse {
                name: label.clone(),
                label,
                count,
            })
            .collect();
        tools.sort_by(|a, b| b.count.cmp(&a.count).then(a.label.cmp(&b.label)));

        let state = if live {
            "working now".to_string()
        } else {
            format!("finished {}", say::ago(now, row.ended_at_ms))
        };

        sessions_out.push(Session {
            id: row.sid.to_string(),
            agent_label: say::agent_noun(&row.agent).to_string(),
            agent: row.agent.clone(),
            objective: if row.objective.is_empty() {
                "no instruction was recorded".to_string()
            } else {
                row.objective.clone()
            },
            model: row.model.clone(),
            when: say::ago(now, row.ended_at_ms),
            at_ms: row.ended_at_ms,
            // Wall time from first record to last, which is not the same
            // as time spent working — say "spanned" and mean it.
            ran_for: say::span(row.started_at_ms, row.ended_at_ms),
            turns: row.turns,
            tools,
            live,
            state,
            outcome: row.outcome.as_deref().map(|o| say::outcome(o).to_string()),
            more_touched: touched_all.len().saturating_sub(touched.len()),
            touched,
            produced,
            features: query::features_of(loaded, &row.sid),
        });
    }

    // Intervene or trust: the two warnings that decision needs, ranked
    // collision first. A signal can only be as fresh as the last import,
    // and each sentence carries that caveat.
    let mut signals: Vec<Concern> = Vec::new();
    for (file, names) in &live_files {
        if names.len() < 2 {
            continue;
        }
        let label = query::make_ref(index, store, file).label;
        let (title, reason) = say::collision(&label, names);
        signals.push(Concern {
            severity: "act".to_string(),
            title,
            reason,
            fix_command: Some(format!("brain sessions import . --prefix {prefix}")),
            target: Some(query::make_ref(index, store, file)),
            repeats: 1,
            also: Vec::new(),
        });
    }
    for (agent, ran_for) in &maybe_stuck {
        let (title, reason) = say::stuck(agent, ran_for);
        signals.push(Concern {
            severity: "watch".to_string(),
            title,
            reason,
            fix_command: Some(format!("brain sessions import . --prefix {prefix}")),
            target: None,
            repeats: 1,
            also: Vec::new(),
        });
    }

    let approvals = approvals(loaded)?;
    // The desk shows proposed changes in full; listing them again below
    // would be the same decision twice.
    let changes: Vec<WorkItem> = changes(loaded)?
        .into_iter()
        .filter(|c| c.stage != "proposed")
        .collect();
    let plans = plans(loaded)?;

    let live_count = sessions_out.iter().filter(|s| s.live).count();
    let headline = if live_count > 0 {
        format!(
            "{} working here right now.",
            say::count(live_count as u64, "agent is", "agents are")
        )
    } else if !approvals.is_empty() {
        format!(
            "{} waiting for your decision.",
            say::count(approvals.len() as u64, "change is", "changes are")
        )
    } else if !changes.is_empty() {
        format!(
            "{} waiting to be finished.",
            say::count(changes.len() as u64, "governed change is", "governed changes are")
        )
    } else if let Some(latest) = sessions_out.first() {
        format!("Nothing is in flight. The last agent finished {}.", latest.when)
    } else {
        "Nothing is in flight.".to_string()
    };

    let sessions_hint = sessions_out.is_empty().then(|| {
        "No agent sessions have been recorded here yet. Import them to see what Claude Code and Codex did in this workspace.".to_string()
    });
    let sessions_hint_command = sessions_hint
        .is_some()
        .then(|| format!("brain sessions import . --prefix {prefix}"));

    // Handed back and forth: the same file edited by more than one
    // session. Derived from the touched edges the surface already reads.
    let mut rework_ranked: Vec<(usize, crate::dto::Fact)> = Vec::new();
    for (file, who) in &editors {
        if who.len() < 2 {
            continue;
        }
        let reference = query::make_ref(index, store, file);
        rework_ranked.push((
            who.len(),
            Fact {
                text: format!(
                    "{} was edited by {}",
                    reference.label,
                    say::count(who.len() as u64, "session", "different sessions")
                ),
                reason: Some("handed back and forth — worth asking why".to_string()),
                tone: "watch".to_string(),
                target: Some(reference),
            },
        ));
    }
    rework_ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.text.cmp(&b.1.text)));
    let rework: Vec<Fact> = rework_ranked.into_iter().take(6).map(|(_, f)| f).collect();

    Ok(WorkView {
        snapshot: loaded.snapshot.clone(),
        headline,
        signals,
        approvals,
        sessions: sessions_out,
        changes,
        plans,
        sessions_hint,
        sessions_hint_command,
        rework,
    })
}

/// Proposed changes waiting for a person, oldest first — the decision
/// that has waited longest leads. Each carries the recorded diff, the
/// pre-apply briefing of its target, and the command that applies it:
/// eyes shows the decision, the CLI executes it.
pub(crate) fn approvals(loaded: &Loaded) -> Result<Vec<Approval>, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let now = loaded.snapshot.generated_at_ms;

    let mut out = Vec::new();
    for (sid, labels) in query::scoped(index, store, prefix, "change")? {
        let (at_ms, status) = twin::latest_at(index, store, &sid, "status")
            .map_err(|e| e.to_string())?
            .unwrap_or((0, String::new()));
        if status != "proposed" {
            continue;
        }
        let slug = labels.get("slug").cloned().unwrap_or_default();
        let target = labels.get("target").cloned().unwrap_or_default();
        let reason = twin::latest(index, store, &sid, "reason")
            .map_err(|e| e.to_string())?
            .or_else(|| labels.get("title").cloned())
            .unwrap_or_default();
        let move_to = twin::latest(index, store, &sid, "move_to").map_err(|e| e.to_string())?;
        let before = twin::latest(index, store, &sid, "before_content").map_err(|e| e.to_string())?;
        let after = twin::latest(index, store, &sid, "content").map_err(|e| e.to_string())?;

        let (summary, diff, diff_note) = if let Some(to) = move_to {
            (say::change_moves(&target, &to), Vec::new(), None)
        } else {
            let after = after.unwrap_or_default();
            let created = before.is_none();
            let (rows, gone, added, note) = diff_rows(before.as_deref().unwrap_or(""), &after);
            (say::change_summary(gone, added, created), rows, note)
        };

        let file_sid = brain_core::ids::StableId::derive(&["file", &target]);
        let briefing =
            super::thing::briefing_rows(loaded, &file_sid, &target, now).unwrap_or_default();

        out.push(Approval {
            id: sid.to_string(),
            target,
            reason,
            when: say::ago(now, at_ms),
            at_ms,
            summary,
            diff,
            diff_note,
            briefing,
            apply_command: format!("brain change apply {prefix} {slug} --cap fs"),
            features: query::features_of(loaded, &sid),
        });
    }
    out.sort_by(|a, b| a.at_ms.cmp(&b.at_ms).then(a.target.cmp(&b.target)));
    Ok(out)
}

/// The recorded diff of a change, trimmed to what moved: the shared
/// head and tail are dropped (two lines of each kept for footing), the
/// middle renders removed-then-added, and anything the cap hides is
/// counted out loud. Returns (rows, lines gone, lines added, note).
pub(crate) fn diff_rows(before: &str, after: &str) -> (Vec<DiffRow>, usize, usize, Option<String>) {
    const CONTEXT: usize = 2;
    const CAP: usize = 60;
    let b: Vec<&str> = before.lines().collect();
    let a: Vec<&str> = after.lines().collect();
    let mut head = 0;
    while head < b.len() && head < a.len() && b[head] == a[head] {
        head += 1;
    }
    let mut tail = 0;
    while tail < b.len() - head && tail < a.len() - head && b[b.len() - 1 - tail] == a[a.len() - 1 - tail]
    {
        tail += 1;
    }
    let gone = &b[head..b.len() - tail];
    let added = &a[head..a.len() - tail];

    let row = |kind: &str, text: &str| DiffRow {
        kind: kind.to_string(),
        text: text.to_string(),
    };
    let mut rows = Vec::new();
    for line in &b[head.saturating_sub(CONTEXT)..head] {
        rows.push(row("same", line));
    }
    let mut hidden = 0;
    for (i, line) in gone.iter().enumerate() {
        if i < CAP {
            rows.push(row("gone", line));
        } else {
            hidden += 1;
        }
    }
    for (i, line) in added.iter().enumerate() {
        if i < CAP {
            rows.push(row("new", line));
        } else {
            hidden += 1;
        }
    }
    for line in &a[a.len() - tail..(a.len() - tail + CONTEXT).min(a.len())] {
        rows.push(row("same", line));
    }
    let note = (hidden > 0).then(|| {
        format!(
            "{} of the changed lines are not shown here",
            hidden
        )
    });
    (rows, gone.len(), added.len(), note)
}

/// Governed changes that have not reached a settled state.
pub(crate) fn changes(loaded: &Loaded) -> Result<Vec<WorkItem>, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let now = loaded.snapshot.generated_at_ms;

    let mut out = Vec::new();
    for (sid, labels) in query::scoped(index, store, prefix, "change")? {
        let (at_ms, status) = twin::latest_at(index, store, &sid, "status")
            .map_err(|e| e.to_string())?
            .unwrap_or((0, String::new()));
        let (stage, note) = say::change_stage(&status);
        let slug = labels.get("slug").cloned().unwrap_or_default();
        let tone = match status.as_str() {
            "verified" | "reverted" => "good",
            "broken" | "failed" | "indeterminate" => "bad",
            _ => "watch",
        };
        // A verified or reverted change is history, not work.
        if matches!(status.as_str(), "verified" | "reverted") {
            continue;
        }
        out.push(WorkItem {
            id: sid.to_string(),
            title: labels
                .get("title")
                .cloned()
                .unwrap_or_else(|| labels.get("target").cloned().unwrap_or(slug.clone())),
            kind: "change".to_string(),
            noun: say::kind_noun("change").to_string(),
            glyph: say::kind_glyph("change").to_string(),
            stage: stage.to_string(),
            note: note.to_string(),
            tone: tone.to_string(),
            when: say::ago(now, at_ms),
            at_ms,
            fix_command: Some(match status.as_str() {
                "proposed" => format!("brain change apply {prefix} {slug} --cap fs"),
                "applied" => format!("brain change verify {prefix} {slug}"),
                "indeterminate" => "brain recover".to_string(),
                _ => format!("brain change show {prefix} {slug}"),
            }),
            features: query::features_of(loaded, &sid),
        });
    }
    out.sort_by(|a, b| b.at_ms.cmp(&a.at_ms));
    Ok(out)
}

/// Plans that are still open.
pub(crate) fn plans(loaded: &Loaded) -> Result<Vec<WorkItem>, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let now = loaded.snapshot.generated_at_ms;

    let mut out = Vec::new();
    for kind in ["plan", "task_list"] {
        for (sid, labels) in query::scoped(index, store, prefix, kind)? {
            let (state, _) = lifecycle::of(index, store, &sid).map_err(|e| e.to_string())?;
            if !state.is_active() {
                continue;
            }
            let at_ms = query::changed_at(index, store, &sid);
            let slug = labels.get("slug").cloned().unwrap_or_default();
            let mentions = twin::live_from(index, store, &sid, "mentions")
                .map_err(|e| e.to_string())?
                .len();
            out.push(WorkItem {
                id: sid.to_string(),
                title: query::display_name(index, store, &sid, &labels),
                kind: kind.to_string(),
                noun: say::kind_noun(kind).to_string(),
                glyph: say::kind_glyph(kind).to_string(),
                stage: "open".to_string(),
                note: if mentions > 0 {
                    format!("touches {}", say::count(mentions as u64, "file", "files"))
                } else {
                    "nothing links it to the code yet".to_string()
                },
                tone: "watch".to_string(),
                when: say::ago(now, at_ms),
                at_ms,
                fix_command: Some(format!("brain plan done {prefix} {slug}")),
                features: query::features_of(loaded, &sid),
            });
        }
    }
    out.sort_by(|a, b| b.at_ms.cmp(&a.at_ms));
    Ok(out)
}

/// Tool names are the agent's vocabulary; say what the tool did.
fn tool_label(name: &str) -> String {
    match name {
        "Edit" | "Write" | "NotebookEdit" | "apply_patch" => "edited files",
        "Read" => "read files",
        "Bash" | "exec_command" | "exec" | "shell" => "ran commands",
        "Grep" | "Glob" | "rg" => "searched",
        "Agent" | "Task" => "delegated",
        "WebFetch" | "WebSearch" => "looked things up",
        "TaskCreate" | "TaskUpdate" | "update_plan" => "tracked work",
        other if other.starts_with("mcp__") => "used a connected tool",
        _ => "other tools",
    }
    .to_string()
}
