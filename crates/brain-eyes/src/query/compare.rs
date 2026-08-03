//! "What was true then, and what changed since?"
//!
//! Two moments through one code path: the past and the present are
//! computed by the same function at different `t`, so the sides are
//! comparable by construction. Time travel is keyed by cause — commits
//! the twin saw as HEAD, and named baselines — never a bare clock. What
//! a past moment cannot honestly show (the working tree, attention,
//! contradictions) is left out and *said* to be left out.

use crate::dto::*;
use crate::say;
use crate::state::Loaded;
use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::Index;
use brain_observe::{baseline, features, testing, twin};
use std::collections::{BTreeMap, BTreeSet};

/// The moments a person can return to, newest first. Commits resolve by
/// their exact recorded moment (the value is the epoch), so the picker
/// and the diff can never disagree about when "then" was.
pub fn moments(loaded: &Loaded) -> Result<MomentsView, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let now = loaded.snapshot.generated_at_ms;

    let mut moments: Vec<MomentRef> = Vec::new();
    for b in baseline::list(store, index, prefix).map_err(|e| e.to_string())? {
        moments.push(MomentRef {
            value: b.name.clone(),
            kind: "baseline".to_string(),
            label: b.name.clone(),
            at_ms: b.at_ms,
            when: say::ago(now, b.at_ms),
        });
    }
    let repo = StableId::derive(&["repo", prefix]);
    for id in index.observations_of(&repo) {
        if let Object::Observation {
            property,
            value,
            observed_at_ms,
            ..
        } = store.get(&id).map_err(|e| e.to_string())?
        {
            if property == "git_commit" {
                let short: String = value.chars().take(7).collect();
                moments.push(MomentRef {
                    value: observed_at_ms.to_string(),
                    kind: "commit".to_string(),
                    label: format!("commit {short}"),
                    at_ms: observed_at_ms,
                    when: say::ago(now, observed_at_ms),
                });
            }
        }
    }
    moments.sort_by(|a, b| b.at_ms.cmp(&a.at_ms).then(a.label.cmp(&b.label)));
    let headline = if moments.is_empty() {
        "Nothing to return to yet — commits and baselines appear here as the graph records them."
            .to_string()
    } else {
        format!(
            "{} worth returning to.",
            say::count(moments.len() as u64, "moment", "moments")
        )
    };
    Ok(MomentsView {
        snapshot: loaded.snapshot.clone(),
        headline,
        moments,
    })
}

/// The comparison: `from` is "then", `to` is "now" (usually `live`).
pub fn build(loaded: &Loaded, from: &str, to: &str) -> Result<CompareView, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let now = loaded.snapshot.generated_at_ms;

    let from_t = twin::resolve_when(store, index, prefix, from)?;
    let to_t = twin::resolve_when(store, index, prefix, to)?;

    let then_side = state_at(loaded, from_t)?;
    let now_side = state_at(loaded, to_t)?;
    let then_moment = moment_ref(loaded, from, from_t, now)?;
    let vs_moment = moment_ref(loaded, to, to_t, now)?;

    // Feature diff. Rank orders how much a feature has: 2 ready, 1
    // partly backed, 0 nothing. A rank fall is a regression; a fall in
    // met checks at the same rank is a smaller one. A done-flip always
    // sorts above a met-count slip.
    let rank = |s: &FeatureState| -> i64 {
        if s.done {
            2
        } else if s.met > 0 {
            1
        } else {
            0
        }
    };
    let mut regressions: Vec<(i64, FeatureDelta)> = Vec::new();
    let mut improvements: Vec<FeatureDelta> = Vec::new();
    let mut appeared: Vec<FeatureDelta> = Vec::new();
    let mut removed: Vec<FeatureDelta> = Vec::new();
    let slugs: BTreeSet<&String> = then_side
        .features
        .keys()
        .chain(now_side.features.keys())
        .collect();
    for slug in slugs {
        match (then_side.features.get(slug), now_side.features.get(slug)) {
            (Some(a), Some(b)) => {
                let (then_rank, now_rank) = (rank(a), rank(b));
                let delta = FeatureDelta {
                    slug: slug.clone(),
                    title: b.title.clone(),
                    sentence: say::feature_moved(a.done, a.met, a.total, b.done, b.met, b.total),
                    tone: String::new(),
                };
                if now_rank < then_rank || (now_rank == then_rank && b.met < a.met) {
                    regressions.push((
                        then_rank - now_rank,
                        FeatureDelta {
                            tone: "bad".to_string(),
                            ..delta
                        },
                    ));
                } else if now_rank > then_rank || (now_rank == then_rank && b.met > a.met) {
                    improvements.push(FeatureDelta {
                        tone: "good".to_string(),
                        ..delta
                    });
                }
            }
            (None, Some(b)) => appeared.push(FeatureDelta {
                slug: slug.clone(),
                title: b.title.clone(),
                sentence: say::feature_appeared(b.done, b.met, b.total),
                tone: "quiet".to_string(),
            }),
            (Some(a), None) => removed.push(FeatureDelta {
                slug: slug.clone(),
                title: a.title.clone(),
                sentence: say::feature_removed().to_string(),
                tone: "quiet".to_string(),
            }),
            (None, None) => unreachable!("slug came from one of the sides"),
        }
    }
    regressions.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.title.cmp(&b.1.title)));
    let regressions: Vec<FeatureDelta> = regressions.into_iter().map(|(_, d)| d).collect();
    improvements.sort_by(|a, b| a.title.cmp(&b.title));
    appeared.sort_by(|a, b| a.title.cmp(&b.title));
    removed.sort_by(|a, b| a.title.cmp(&b.title));

    let metrics = metric_rows(&then_side, &now_side);
    let headline = say::compare_headline(
        regressions.len(),
        improvements.len(),
        appeared.len(),
        removed.len(),
    );
    let banner = (from_t != u64::MAX).then(|| {
        say::asof_banner(&say::moment_phrase(
            now,
            then_moment.at_ms,
            &then_moment.kind,
            &then_moment.label,
        ))
    });
    // Naming a moment goes through the CLI — eyes renders the command
    // and never writes. A baseline already has its name.
    let baseline_command = (then_moment.kind != "baseline" && from_t != u64::MAX)
        .then(|| format!("brain baseline add {prefix} <name> --at {from_t}"));

    Ok(CompareView {
        snapshot: loaded.snapshot.clone(),
        then_moment,
        vs_moment,
        banner,
        headline,
        metrics,
        regressions,
        improvements,
        appeared,
        removed,
        omissions: say::past_omissions().to_string(),
        baseline_command,
    })
}

struct FeatureState {
    title: String,
    done: bool,
    met: usize,
    total: usize,
}

struct SideState {
    features: BTreeMap<String, FeatureState>,
    /// (passed, total) of the last run at or before the moment.
    tests: Option<(usize, usize)>,
    files: usize,
}

/// Everything one side of the comparison can honestly know at `t`. The
/// live side is the same function at `u64::MAX` — one code path, two
/// moments.
fn state_at(loaded: &Loaded, t: u64) -> Result<SideState, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();

    let mut feats = BTreeMap::new();
    for row in features::list_at(store, index, prefix, t).map_err(|e| e.to_string())? {
        let report =
            features::evaluate_at(store, index, prefix, &row.slug, t).map_err(|e| e.to_string())?;
        let (met, total) = report.score();
        feats.insert(
            row.slug.clone(),
            FeatureState {
                title: row.title,
                done: report.done,
                met,
                total,
            },
        );
    }
    let mut last: Option<(u64, usize, usize)> = None;
    for (at, total, passed, _failed, _format) in
        testing::runs(store, index, prefix).map_err(|e| e.to_string())?
    {
        if at <= t && last.is_none_or(|(best, _, _)| at > best) {
            last = Some((at, passed, total));
        }
    }
    let files = twin::files_at(store, index, prefix, t)
        .map_err(|e| e.to_string())?
        .len();
    Ok(SideState {
        features: feats,
        tests: last.map(|(_, passed, total)| (passed, total)),
        files,
    })
}

/// Describe one side's moment for the chrome: what was picked, and what
/// it meant.
fn moment_ref(loaded: &Loaded, raw: &str, t: u64, now: u64) -> Result<MomentRef, String> {
    if t == u64::MAX {
        return Ok(MomentRef {
            value: "live".to_string(),
            kind: "live".to_string(),
            label: "now".to_string(),
            at_ms: now,
            when: "now".to_string(),
        });
    }
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    if let Some(b) = baseline::list(store, index, prefix)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|b| b.name == raw)
    {
        return Ok(MomentRef {
            value: b.name.clone(),
            kind: "baseline".to_string(),
            label: b.name,
            at_ms: t,
            when: say::ago(now, t),
        });
    }
    // The commit the twin saw as HEAD at that exact moment, if any.
    let repo = StableId::derive(&["repo", prefix]);
    for id in index.observations_of(&repo) {
        if let Object::Observation {
            property,
            value,
            observed_at_ms,
            ..
        } = store.get(&id).map_err(|e| e.to_string())?
        {
            if property == "git_commit" && observed_at_ms == t {
                let short: String = value.chars().take(7).collect();
                return Ok(MomentRef {
                    value: t.to_string(),
                    kind: "commit".to_string(),
                    label: format!("commit {short}"),
                    at_ms: t,
                    when: say::ago(now, t),
                });
            }
        }
    }
    Ok(MomentRef {
        value: t.to_string(),
        kind: "moment".to_string(),
        label: "that moment".to_string(),
        at_ms: t,
        when: say::ago(now, t),
    })
}

fn metric_rows(then_side: &SideState, now_side: &SideState) -> Vec<MetricDelta> {
    let mut out = Vec::new();

    let ready = |s: &SideState| s.features.values().filter(|f| f.done).count();
    let (then_ready, now_ready) = (ready(then_side), ready(now_side));
    out.push(MetricDelta {
        label: "Features ready".to_string(),
        then_value: format!("{} of {}", then_ready, then_side.features.len()),
        now_value: format!("{} of {}", now_ready, now_side.features.len()),
        sentence: say::ready_delta(
            then_ready,
            then_side.features.len(),
            now_ready,
            now_side.features.len(),
        ),
        tone: direction_tone(then_ready as i64, now_ready as i64, true),
    });

    let failing = |tests: Option<(usize, usize)>| tests.map(|(p, t)| (t - p) as i64);
    let tone = match (failing(then_side.tests), failing(now_side.tests)) {
        (Some(a), Some(b)) => direction_tone(a, b, false),
        _ => "quiet".to_string(),
    };
    let side_value = |tests: Option<(usize, usize)>| match tests {
        None => "no run".to_string(),
        Some((p, t)) => format!("{p} of {t}"),
    };
    out.push(MetricDelta {
        label: "Tests passing".to_string(),
        then_value: side_value(then_side.tests),
        now_value: side_value(now_side.tests),
        sentence: say::tests_delta(then_side.tests, now_side.tests),
        tone,
    });

    out.push(MetricDelta {
        label: "Files".to_string(),
        then_value: then_side.files.to_string(),
        now_value: now_side.files.to_string(),
        sentence: say::files_delta(then_side.files, now_side.files),
        tone: "quiet".to_string(),
    });
    out
}

/// Judge a move: for measures where more is better, a fall is bad; for
/// debts, a rise is.
fn direction_tone(then_n: i64, now_n: i64, higher_is_better: bool) -> String {
    let tone = if now_n == then_n {
        "quiet"
    } else if (now_n > then_n) == higher_is_better {
        "good"
    } else {
        "bad"
    };
    tone.to_string()
}
