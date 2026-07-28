//! "What is being worked on?" — with, for the first time, an answer that
//! names who.
//!
//! Until agent sessions were ingested this surface could only have shown
//! governed changes and open plans, which in a healthy repository is
//! usually nothing at all. A session is the graph's only record of a
//! principal: what it was asked to do, which model, how long it ran, and
//! which files it changed.
//!
//! What is *not* here: approvals, authorisation queues, and pending
//! decisions. The graph models none of them, so nothing appears to
//! explain their absence.

use crate::dto::*;
use crate::query;
use crate::say;
use crate::state::Loaded;
use brain_observe::{lifecycle, sessions, twin};

/// A session whose last recorded activity is inside this window is still
/// considered to be running.
const LIVE_WITHIN_MS: u64 = 20 * 60 * 1000;

pub fn build(loaded: &Loaded) -> Result<WorkView, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let now = loaded.snapshot.generated_at_ms;

    let rows = sessions::list(store, index, prefix).map_err(|e| e.to_string())?;
    let mut sessions_out = Vec::new();
    for row in rows.iter().take(25) {
        let live = now.saturating_sub(row.ended_at_ms) < LIVE_WITHIN_MS;
        let touched_all = twin::live_from(index, store, &row.sid, "touched")
            .map_err(|e| e.to_string())?;
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
            more_touched: touched_all.len().saturating_sub(touched.len()),
            touched,
            produced,
            features: query::features_of(loaded, &row.sid),
        });
    }

    let changes = changes(loaded)?;
    let plans = plans(loaded)?;

    let live_count = sessions_out.iter().filter(|s| s.live).count();
    let headline = if live_count > 0 {
        format!(
            "{} working here right now.",
            say::count(live_count as u64, "agent is", "agents are")
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

    Ok(WorkView {
        snapshot: loaded.snapshot.clone(),
        headline,
        sessions: sessions_out,
        changes,
        plans,
        sessions_hint,
        sessions_hint_command,
    })
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
