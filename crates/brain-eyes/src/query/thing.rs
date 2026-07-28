//! "What is this, can I trust it, and what breaks if I touch it?"
//!
//! One page for every kind. The body comes first — a decision is something
//! you read, not a row you inspect — then the judgments about it, then its
//! neighbourhood laid out so position means something: what it depends on
//! to the left, what depends on it to the right.

use crate::body;
use crate::dto::*;
use crate::query;
use crate::say;
use crate::state::{EventPayload, Loaded};
use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::Index;
use brain_observe::{features, lifecycle, twin};
use std::collections::{BTreeMap, BTreeSet};

/// Predicates that describe dependency rather than annotation.
const STRUCTURAL: &[&str] = &["imports", "contains", "changes", "renamed_to"];

pub fn build(loaded: &Loaded, id: &str, content_root: Option<&std::path::Path>) -> Result<ThingView, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let now = loaded.snapshot.generated_at_ms;
    let sid = StableId(id.to_string());

    if index.entity_nodes(&sid).is_empty() {
        return Err("this entity is not in the current graph".to_string());
    }
    let labels = query::labels_of(index, store, &sid);
    let kind = query::kind_of(index, store, &sid).unwrap_or_default();
    let label = twin::sid_label(index, store, &sid);
    let title = query::display_name(index, store, &sid, &labels);
    let slug = labels.get("slug").cloned().unwrap_or_default();

    // ---- state -----------------------------------------------------------
    let (state, why) = lifecycle::of(index, store, &sid).map_err(|e| e.to_string())?;
    let insights = loaded.insights();
    let stale = insights
        .stale_docs
        .iter()
        .find(|doc| doc.kind == kind && doc.slug == slug);

    let (state_text, state_note, tone) = if let Some(sentence) =
        say::lifecycle(state.as_str(), &why)
    {
        (Some(sentence), None, "quiet".to_string())
    } else {
        let (word, note) = say::freshness(
            stale.map(|doc| doc.severity.as_str()),
            state.is_active(),
        );
        let tone = match word {
            "may be wrong" => "watch",
            "current" => "good",
            _ => "quiet",
        };
        (
            Some(word.to_string()),
            Some(note.to_string()),
            tone.to_string(),
        )
    };

    // ---- judgments -------------------------------------------------------
    let mut facts: Vec<Fact> = Vec::new();
    if let Some(doc) = stale {
        for changed in &doc.changed {
            let target = StableId::derive(&["file", changed]);
            facts.push(Fact {
                text: format!("{changed} changed after this was written"),
                reason: None,
                tone: if doc.severity == twin::Severity::Warn {
                    "watch".to_string()
                } else {
                    "quiet".to_string()
                },
                target: Some(query::make_ref(index, store, &target)),
            });
        }
    }
    if let Ok(Some(reviewed)) = twin::latest_at(index, store, &sid, "reviewed") {
        facts.push(Fact {
            text: format!("reviewed {}", say::ago(now, reviewed.0)),
            reason: Some(reviewed.1),
            tone: "good".to_string(),
            target: None,
        });
    }
    if let Ok(Some(missing)) = twin::latest(index, store, &sid, "missing") {
        if !missing.trim().is_empty() {
            facts.push(Fact {
                text: format!("does not meet its contract: missing {missing}"),
                reason: Some("the contract for this kind asks for these fields".to_string()),
                tone: "watch".to_string(),
                target: None,
            });
        }
    }
    for item in loaded.attention().iter().filter(|item| item.label == label) {
        for raw in &item.reasons {
            if let Some(text) = say::attention_reason(raw) {
                facts.push(Fact {
                    text,
                    reason: None,
                    tone: "quiet".to_string(),
                    target: None,
                });
            }
        }
    }

    // ---- relations, neighbourhood ---------------------------------------
    let live = live_relations(loaded, &sid)?;
    let mut relations: Vec<RelationLink> = Vec::new();
    for (predicate, other, outgoing) in &live {
        relations.push(RelationLink {
            phrase: say::predicate_phrase(predicate, *outgoing),
            predicate: predicate.clone(),
            outgoing: *outgoing,
            other: query::make_ref(index, store, other),
        });
    }
    relations.sort_by(|a, b| a.phrase.cmp(&b.phrase).then(a.other.label.cmp(&b.other.label)));
    let neighborhood = neighborhood(loaded, &sid, &live)?;

    // ---- body, versions, history ----------------------------------------
    let (body, body_error) = match body::resolve(loaded, &sid, &kind, &labels, content_root) {
        Ok(resolved) => (Some(resolved.view), None),
        Err(error) => (None, Some(error)),
    };
    let versions = versions(loaded, &sid, now)?;
    let history = history(loaded, &sid, now, 40);
    let extras = extras(loaded, &sid, &kind, &slug, &live, now)?;

    let mut details = vec![
        ("Stable id".to_string(), sid.to_string()),
        ("Kind".to_string(), kind.clone()),
    ];
    if let Some(path) = labels.get("path") {
        details.push(("Workspace path".to_string(), path.clone()));
    }
    if let Ok(Some(hash)) = twin::latest(index, store, &sid, "content_b3") {
        details.push(("Content identity".to_string(), hash));
    }

    Ok(ThingView {
        snapshot: loaded.snapshot.clone(),
        id: sid.to_string(),
        label,
        title,
        kind: kind.clone(),
        noun: say::kind_noun(&kind).to_string(),
        glyph: say::kind_glyph(&kind).to_string(),
        state: state_text,
        state_note,
        tone,
        facts,
        body,
        body_error,
        neighborhood,
        relations,
        versions,
        history,
        extras,
        details,
    })
}

/// Every live edge touching this entity, from the shared event scan.
/// `(predicate, other, outgoing)`.
fn live_relations(
    loaded: &Loaded,
    sid: &StableId,
) -> Result<Vec<(String, StableId, bool)>, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let mut seen: BTreeSet<(String, String, bool)> = BTreeSet::new();
    let mut out = Vec::new();
    for row in loaded.events() {
        let EventPayload::Relation { predicate, to } = &row.payload else {
            continue;
        };
        let Some(from) = &row.subject else { continue };
        let (other, outgoing) = if from == sid {
            (to.clone(), true)
        } else if to == sid {
            (from.clone(), false)
        } else {
            continue;
        };
        let key = (predicate.clone(), other.to_string(), outgoing);
        if !seen.insert(key) {
            continue;
        }
        let (a, b) = if outgoing {
            (sid.clone(), other.clone())
        } else {
            (other.clone(), sid.clone())
        };
        if !brain_index::edge_active(&**index, store, &a, predicate, &b)
            .map_err(|e| e.to_string())?
        {
            continue;
        }
        out.push((predicate.clone(), other, outgoing));
    }
    Ok(out)
}

fn neighborhood(
    loaded: &Loaded,
    sid: &StableId,
    live: &[(String, StableId, bool)],
) -> Result<Neighborhood, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    const SHOWN: usize = 10;

    let mut upstream = Vec::new();
    let mut downstream = Vec::new();
    let mut tests = Vec::new();
    let mut docs = Vec::new();
    let mut decisions = Vec::new();

    for (predicate, other, outgoing) in live {
        let other_kind = query::kind_of(index, store, other).unwrap_or_default();
        let entry = query::make_ref(index, store, other);
        match (other_kind.as_str(), predicate.as_str()) {
            ("test_case" | "test_run", _) => tests.push(entry),
            (_, "covers") if !*outgoing => tests.push(entry),
            ("decision", _) => decisions.push(entry),
            ("doc" | "runbook" | "plan" | "skill" | "agent_config" | "task_list", _) => {
                docs.push(entry)
            }
            _ if STRUCTURAL.contains(&predicate.as_str()) => {
                if *outgoing {
                    upstream.push(entry)
                } else {
                    downstream.push(entry)
                }
            }
            _ => {
                if *outgoing {
                    upstream.push(entry)
                } else {
                    downstream.push(entry)
                }
            }
        }
    }

    let dedup = |mut list: Vec<Ref>| {
        list.sort_by(|a, b| a.label.cmp(&b.label));
        list.dedup_by(|a, b| a.id == b.id);
        list
    };
    let (mut upstream, mut downstream) = (dedup(upstream), dedup(downstream));
    let (upstream_total, downstream_total) = (upstream.len(), downstream.len());
    upstream.truncate(SHOWN);
    downstream.truncate(SHOWN);

    let sentence = match (upstream_total, downstream_total) {
        (0, 0) => "Nothing links to this yet.".to_string(),
        (0, d) => format!(
            "{} depend{} on this; it depends on nothing.",
            say::count(d as u64, "thing", "things"),
            if d == 1 { "s" } else { "" }
        ),
        (u, 0) => format!(
            "This uses {}; nothing depends on it yet.",
            say::count(u as u64, "thing", "things")
        ),
        (u, d) => format!(
            "This uses {}, and {} depend{} on it — edits here reach them.",
            say::count(u as u64, "thing", "things"),
            say::count(d as u64, "thing", "things"),
            if d == 1 { "s" } else { "" }
        ),
    };

    Ok(Neighborhood {
        center: query::make_ref(index, store, sid),
        upstream,
        downstream,
        tests: dedup(tests).into_iter().take(SHOWN).collect(),
        docs: dedup(docs).into_iter().take(SHOWN).collect(),
        decisions: dedup(decisions).into_iter().take(SHOWN).collect(),
        upstream_total,
        downstream_total,
        sentence,
    })
}

fn versions(loaded: &Loaded, sid: &StableId, now: u64) -> Result<Vec<Version>, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let mut rows: Vec<(u64, String)> = Vec::new();
    for node in index.observations_of(sid) {
        if let Ok(Object::Observation {
            property,
            value,
            observed_at_ms,
            ..
        }) = store.get(&node)
        {
            if property == "content_b3" {
                rows.push((observed_at_ms, value));
            }
        }
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(rows
        .into_iter()
        .enumerate()
        .map(|(position, (at_ms, hash))| Version {
            at_ms,
            when: say::ago(now, at_ms),
            hash: hash.chars().take(12).collect(),
            note: if position == 0 {
                "the version the graph knows".to_string()
            } else {
                "replaced by a later version".to_string()
            },
            current: position == 0,
        })
        .take(12)
        .collect())
}

fn history(loaded: &Loaded, sid: &StableId, now: u64, limit: usize) -> Vec<HistoryEntry> {
    let mut out: Vec<HistoryEntry> = Vec::new();
    for row in loaded.events().iter().rev() {
        if out.len() >= limit {
            break;
        }
        if row.subject.as_ref() != Some(sid) {
            continue;
        }
        let (text, detail) = match &row.payload {
            EventPayload::Observation { property, value } => match property.as_str() {
                "content_b3" => ("the file changed".to_string(), None),
                "content" => ("the text was rewritten".to_string(), None),
                "present" if value == "false" => ("removed from the workspace".to_string(), None),
                "present" => ("present in the workspace".to_string(), None),
                "status" => (format!("status became {value}"), None),
                "result" => (format!("test result: {value}"), None),
                "conforms" if value == "false" => {
                    ("stopped meeting its contract".to_string(), None)
                }
                "conforms" => ("meets its contract".to_string(), None),
                "reviewed" => ("reviewed and confirmed accurate".to_string(), Some(value.clone())),
                "lifecycle" => (format!("marked {value}"), None),
                "note" => ("a note was left".to_string(), Some(value.clone())),
                "memory" => ("consolidated into memory".to_string(), Some(value.clone())),
                "active" => (
                    if value == "true" {
                        "a link was restored".to_string()
                    } else {
                        "a link was retracted".to_string()
                    },
                    None,
                ),
                other => (format!("{} recorded", other.replace('_', " ")), None),
            },
            EventPayload::Relation { predicate, .. } => (
                format!("linked: {}", say::predicate_phrase(predicate, true)),
                None,
            ),
            _ => continue,
        };
        out.push(HistoryEntry {
            at_ms: row.at_ms,
            when: say::ago(now, row.at_ms),
            text,
            source: source_noun(&row.source).to_string(),
            detail,
        });
    }
    out
}

/// Observation sources are internal words; these are what they mean.
fn source_noun(source: &str) -> &'static str {
    match source {
        "twin" => "observed in the workspace",
        "agent" => "recorded by an agent",
        "seed" => "shipped with brain",
        "sleep" => "consolidation",
        "testrun" | "test" => "test run",
        "govern" => "governed change",
        "docsgen" | "projection" => "generated",
        "tidy" => "tidy",
        "backfill" => "git history",
        "claude-code" => "recorded by an agent",
        "dod" => "definition of done",
        "hook" => "git hook",
        _ => "recorded",
    }
}

/// The audit record for a governed change: what the graph holds about
/// the action, in the order it happened.
///
/// The briefing asks for requester, interface and inputs. The graph
/// records none of the first two — an `Intent` has four fields and a
/// principal is not among them — so this says so rather than inventing an
/// actor. What it *can* show is real: the exact before and after bytes,
/// the capability that was required, the receipt, and the run that
/// verified it. The command is reconstructed from the change's own
/// labels, and is labelled as such.
fn audit(
    loaded: &Loaded,
    sid: &StableId,
    slug: &str,
    status: &str,
) -> Result<Vec<AuditEntry>, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let mut out = Vec::new();

    let recorded = |label: &str, value: String, note: Option<String>| AuditEntry {
        label: label.to_string(),
        value,
        note,
        recorded: true,
    };

    if let Some(reason) = twin::latest(index, store, sid, "reason").map_err(|e| e.to_string())? {
        out.push(recorded("Why", reason, None));
    }
    if let Some(target) = twin::latest(index, store, sid, "target").map_err(|e| e.to_string())? {
        out.push(recorded("What it touched", target, None));
    }

    // Before and after are content hashes; the bytes themselves are shown
    // separately as a diff.
    let short = |hash: String| hash.chars().take(12).collect::<String>();
    if let Some(before) = twin::latest(index, store, sid, "before_b3").map_err(|e| e.to_string())? {
        out.push(recorded(
            "The file before",
            if before == "absent" {
                "the file did not exist".to_string()
            } else {
                short(before)
            },
            Some("the exact bytes are recorded, not just this fingerprint".to_string()),
        ));
    }
    if let Some(after) = twin::latest(index, store, sid, "after_b3").map_err(|e| e.to_string())? {
        out.push(recorded("The file after", short(after), None));
    }

    out.push(recorded(
        "Authority required",
        "fs — permission to write the workspace".to_string(),
        Some("no ambient authority: the write is refused without it".to_string()),
    ));

    // The Intent id is observed onto the change when the effect is
    // attempted; the Receipt is reachable from the intent.
    if let Some(intent) = twin::latest(index, store, sid, "intent").map_err(|e| e.to_string())? {
        out.push(recorded(
            "Intent",
            "recorded before the file was touched".to_string(),
            Some("this ordering is what makes a crash recoverable".to_string()),
        ));
        if let Ok(node) = brain_core::ids::NodeId::parse(&intent) {
            for receipt_node in index.receipts_for(&node) {
                if let Ok(Object::Receipt { ok, detail, .. }) = store.get(&receipt_node) {
                    out.push(recorded(
                        "Receipt",
                        if ok {
                            format!("confirmed: {detail}")
                        } else {
                            format!("failed: {detail}")
                        },
                        None,
                    ));
                }
            }
        }
    }

    let (_, note) = say::change_stage(status);
    if !note.is_empty() {
        out.push(recorded("Outcome", note.to_string(), None));
    }

    out.push(AuditEntry {
        label: "The same thing from a terminal".to_string(),
        value: match status {
            "proposed" => format!("brain change apply {prefix} {slug} --cap fs"),
            "applied" => format!("brain change verify {prefix} {slug}"),
            _ => format!("brain change show {prefix} {slug}"),
        },
        note: Some(say::RECONSTRUCTED.to_string()),
        recorded: false,
    });
    Ok(out)
}

fn extras(
    loaded: &Loaded,
    sid: &StableId,
    kind: &str,
    slug: &str,
    live: &[(String, StableId, bool)],
    now: u64,
) -> Result<ThingExtras, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let mut extras = ThingExtras::default();

    match kind {
        "change" => {
            let mut timeline: BTreeMap<String, u64> = BTreeMap::new();
            for node in index.observations_of(sid) {
                if let Ok(Object::Observation {
                    property,
                    value,
                    observed_at_ms,
                    ..
                }) = store.get(&node)
                {
                    if property == "status" {
                        timeline.insert(value, observed_at_ms);
                    }
                }
            }
            let reached = |name: &str| timeline.get(name).copied();
            let current = twin::latest(index, store, sid, "status")
                .map_err(|e| e.to_string())?
                .unwrap_or_default();
            for (key, label) in [
                ("proposed", "Written down"),
                ("applied", "Applied to the workspace"),
                ("verified", "Verified by tests"),
            ] {
                let at = reached(key);
                let (_, note) = say::change_stage(key);
                extras.stages.push(Stage {
                    label: label.to_string(),
                    note: note.to_string(),
                    state: if at.is_some() {
                        "done".to_string()
                    } else if current == key {
                        "current".to_string()
                    } else {
                        "todo".to_string()
                    },
                    when: at.map(|at| say::ago(now, at)),
                });
            }
            if matches!(current.as_str(), "broken" | "failed" | "reverted" | "indeterminate") {
                let (stage, note) = say::change_stage(&current);
                extras.stages.push(Stage {
                    label: format!("Ended: {stage}"),
                    note: note.to_string(),
                    state: "current".to_string(),
                    when: reached(&current).map(|at| say::ago(now, at)),
                });
            }
            extras.before_text = twin::latest(index, store, sid, "before_content")
                .map_err(|e| e.to_string())?;
            extras.after_text =
                twin::latest(index, store, sid, "content").map_err(|e| e.to_string())?;
            extras.audit = audit(loaded, sid, slug, &current)?;
        }
        "feature" => {
            extras.feature = Some(super::features::node(loaded, slug, 0)?);
            extras.reach = feature_reach(loaded, slug);
            let report =
                features::evaluate(store, index, prefix, slug).map_err(|e| e.to_string())?;
            extras.coverage = report
                .checks
                .iter()
                .map(|check| CoverageCell {
                    label: say::dod_label(&check.predicate).to_string(),
                    met: check.count > 0,
                    detail: if check.count > 0 {
                        format!(
                            "{} linked",
                            say::count(check.count as u64, "record", "records")
                        )
                    } else {
                        format!("nothing linked as {}", say::dod_label(&check.predicate))
                    },
                })
                .collect();
        }
        "decision" => {
            for (predicate, other, outgoing) in live {
                if predicate != "supersedes" {
                    continue;
                }
                let entry = query::make_ref(index, store, other);
                if *outgoing {
                    extras.supersedes.push(entry);
                } else {
                    extras.superseded_by.push(entry);
                }
            }
        }
        "test_case" => {
            let mut rows: Vec<(u64, String)> = Vec::new();
            for node in index.observations_of(sid) {
                if let Ok(Object::Observation {
                    property,
                    value,
                    observed_at_ms,
                    ..
                }) = store.get(&node)
                {
                    if property == "result" {
                        rows.push((observed_at_ms, value));
                    }
                }
            }
            rows.sort_by(|a, b| b.0.cmp(&a.0));
            extras.flips = rows
                .into_iter()
                .map(|(at_ms, result)| HistoryEntry {
                    at_ms,
                    when: say::ago(now, at_ms),
                    text: match result.as_str() {
                        "pass" => "started passing".to_string(),
                        "fail" => "started failing".to_string(),
                        other => format!("result became {other}"),
                    },
                    source: "test run".to_string(),
                    detail: None,
                })
                .collect();

            // What the run left behind about this case: the screenshot,
            // the recording, the trace.
            for (_, asset) in twin::live_to(index, store, sid, "attached_to")
                .map_err(|e| e.to_string())?
            {
                let labels = query::labels_of(index, store, &asset);
                let path = labels.get("path").cloned().unwrap_or_default();
                let subtype = twin::latest(index, store, &asset, "subtype")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default();
                extras.attachments.push(Attachment {
                    id: asset.to_string(),
                    label: path.rsplit('/').next().unwrap_or(&path).to_string(),
                    noun: say::attachment_noun(&subtype).to_string(),
                    subtype,
                    path,
                });
            }
            extras.attachments.sort_by(|a, b| a.label.cmp(&b.label));
        }
        "agent_session" => {
            extras.session = crate::query::work::build(loaded)?
                .sessions
                .into_iter()
                .find(|session| session.id == sid.to_string());
        }
        _ => {}
    }

    // Which features this serves — for every kind, not just features. This
    // is the one line that lets any page in the product say what its
    // subject is part of.
    for owned in loaded.spine().features_of(sid) {
        let through = owned
            .through
            .as_ref()
            .map(|file| twin::sid_label(index, store, file));
        extras.serves.push(Attribution {
            target: query::make_ref(index, store, &owned.feature),
            because: say::attribution_because(
                owned.via.as_str(),
                &owned.predicate,
                through.as_deref(),
            ),
        });
    }
    Ok(extras)
}

/// What a feature reaches: what it declares, then what the twin already
/// pointed at those files by itself.
///
/// The two are shown apart because they are different kinds of statement.
/// A declared link is a claim someone made; a reached record is something
/// the graph observed, and it always names the file that carries it.
fn feature_reach(loaded: &Loaded, slug: &str) -> Option<FeatureReachView> {
    let spine = loaded.spine();
    let reach = spine.reach(slug)?;
    let index = &loaded.index;
    let store = &loaded.store;

    let mut groups: Vec<ReachGroup> = Vec::new();
    let mut declared_total = 0usize;
    let mut reached_total = 0usize;
    for (kind, rows) in &reach.by_kind {
        for declared in [true, false] {
            let items: Vec<ReachItem> = rows
                .iter()
                .filter(|row| {
                    matches!(
                        row.via,
                        brain_observe::spine::Via::Declared | brain_observe::spine::Via::Part
                    ) == declared
                })
                .map(|row| ReachItem {
                    target: query::make_ref(index, store, &row.sid),
                    through: row
                        .through
                        .as_ref()
                        .map(|file| query::make_ref(index, store, file)),
                })
                .collect();
            if items.is_empty() {
                continue;
            }
            if declared {
                declared_total += items.len();
            } else {
                reached_total += items.len();
            }
            groups.push(ReachGroup {
                label: say::kind_plural(kind),
                glyph: say::kind_glyph(kind).to_string(),
                declared,
                total: items.len(),
                items,
            });
        }
    }
    // Declared first: the claim, then what stands behind it.
    groups.sort_by(|a, b| b.declared.cmp(&a.declared).then(a.label.cmp(&b.label)));

    Some(FeatureReachView {
        sentence: say::reach_sentence(declared_total, reached_total, reach.files.len()),
        groups,
    })
}
