//! Pictures, recordings, and the narrated tour.
//!
//! The generated tour is the artifact-rot problem told about itself: it is
//! a recording of what the graph said at a moment, and the graph has moved
//! since. Because the narration is *computed* — `brain_observe::tour`
//! builds every sentence from a query — Eyes can recompute it and name the
//! exact sentence that stopped being true, instead of guessing from
//! timestamps.
//!
//! Screenshots carry `rendered_from`: the command that drew them. That is
//! the closest thing the graph has to reproduction provenance, and it is
//! genuinely recorded rather than reconstructed.

use crate::dto::*;
use crate::query;
use crate::say;
use crate::state::Loaded;
use brain_core::ids::StableId;
use brain_observe::{tour, twin};

pub fn build(loaded: &Loaded, content_root: Option<&std::path::Path>) -> Result<MediaView, String> {
    let items = media_items(loaded)?;
    let tour = tour_view(loaded, &items, content_root)?;

    let headline = match (&tour, items.len()) {
        (_, 0) => "Nothing visual has been recorded here yet.".to_string(),
        (Some(t), n) if !t.drift.is_empty() => format!(
            "{n} recorded, and the tour no longer matches the graph in {}.",
            say::count(t.drift.len() as u64, "place", "places")
        ),
        (_, n) => format!(
            "{} recorded.",
            say::count(n as u64, "picture or recording", "pictures and recordings")
        ),
    };

    Ok(MediaView {
        snapshot: loaded.snapshot.clone(),
        headline,
        tour,
        items,
    })
}

/// Every declared asset that is actually media.
pub fn media_items(loaded: &Loaded) -> Result<Vec<MediaItem>, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let now = loaded.snapshot.generated_at_ms;

    // Staleness for assets is already computed the same way as documents.
    let stale: std::collections::BTreeMap<String, Vec<String>> =
        brain_observe::assets::stale(store, index, prefix)
            .map_err(|e| e.to_string())?
            .into_iter()
            .collect();

    let mut out = Vec::new();
    for (sid, labels) in query::scoped(index, store, prefix, "asset")? {
        let subtype = twin::latest(index, store, &sid, "subtype")
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        if !matches!(subtype.as_str(), "image" | "screencast" | "audio") {
            continue;
        }
        let path = labels.get("path").cloned().unwrap_or_default();
        let slug = labels.get("slug").cloned().unwrap_or_default();
        let file = StableId::derive(&["file", &path]);

        let at_ms = twin::latest_at(index, store, &file, "content_b3")
            .map_err(|e| e.to_string())?
            .map(|(at, _)| at)
            .unwrap_or(0);
        let rendered_from =
            twin::latest(index, store, &file, "rendered_from").map_err(|e| e.to_string())?;

        // Two independent staleness signals, in order of certainty.
        let expected = twin::latest(index, store, &file, "expected_b3")
            .map_err(|e| e.to_string())?;
        let actual = twin::latest(index, store, &file, "content_b3")
            .map_err(|e| e.to_string())?;
        let (state, state_note, tone) = if let (Some(expected), Some(actual)) =
            (&expected, &actual)
        {
            if expected != actual {
                (
                    "replaced",
                    "these bytes are not the ones the render produced".to_string(),
                    "bad",
                )
            } else if let Some(changed) = stale.get(&slug) {
                (
                    "out of date",
                    format!("what it shows has changed since: {}", changed.join(", ")),
                    "watch",
                )
            } else {
                ("as rendered", "unchanged since it was produced".to_string(), "good")
            }
        } else if let Some(changed) = stale.get(&slug) {
            (
                "out of date",
                format!("what it shows has changed since: {}", changed.join(", ")),
                "watch",
            )
        } else {
            ("recorded", "captured by hand, not generated".to_string(), "quiet")
        };

        let owner = twin::live_from(index, store, &sid, "attached_to")
            .map_err(|e| e.to_string())?
            .first()
            .map(|(_, to)| query::make_ref(index, store, to))
            .filter(|reference| reference.kind != "repo");
        let depicts: Vec<Ref> = twin::live_from(index, store, &sid, "depicts")
            .map_err(|e| e.to_string())?
            .iter()
            .take(6)
            .map(|(_, to)| query::make_ref(index, store, to))
            .collect();

        out.push(MediaItem {
            id: sid.to_string(),
            label: path.rsplit('/').next().unwrap_or(&path).to_string(),
            noun: say::attachment_noun(&subtype).to_string(),
            subtype,
            path,
            rendered_from,
            when: if at_ms > 0 {
                say::ago(now, at_ms)
            } else {
                String::new()
            },
            at_ms,
            state: state.to_string(),
            state_note,
            tone: tone.to_string(),
            owner,
            depicts,
        });
    }
    out.sort_by(|a, b| b.at_ms.cmp(&a.at_ms).then(a.label.cmp(&b.label)));
    Ok(out)
}

/// The generated tour, if this workspace has one.
fn tour_view(
    loaded: &Loaded,
    items: &[MediaItem],
    content_root: Option<&std::path::Path>,
) -> Result<Option<Tour>, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();

    let narration_file = StableId::derive(&["file", "docs/generated/narration.txt"]);
    let Some(script_text) = twin::latest(index, store, &narration_file, "content")
        .map_err(|e| e.to_string())?
        .or_else(|| read_narration(content_root))
    else {
        return Ok(None);
    };
    let script: Vec<String> = script_text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    if script.is_empty() {
        return Ok(None);
    }

    // The exact claim: recompute the narration and compare it sentence by
    // sentence against what was recorded.
    let drift: Vec<NarrationDrift> =
        tour::narration_drift_from(loaded.insights(), prefix, &script_text)
            .into_iter()
            .map(|d| NarrationDrift {
                recorded: d.recorded,
                current: d.current,
            })
            .collect();

    let find = |name: &str| items.iter().find(|item| item.path.ends_with(name)).cloned();
    let video = find("tour-narrated.webm").or_else(|| find("tour.webm"));

    // Chapters take their words from the sentence about the same topic,
    // never from the sentence in the same position: the script is
    // conditional and runs in its own order.
    let spoken = tour::narration_from(loaded.insights(), prefix);
    let chapters: Vec<Chapter> = tour::chapters(prefix)
        .into_iter()
        .map(|chapter| Chapter {
            id: chapter.id.to_string(),
            title: chapter.title.to_string(),
            command: chapter.command(),
            image: find(&chapter.image),
            narration: chapter.topic.and_then(|topic| {
                spoken
                    .iter()
                    .find(|(tag, _)| *tag == topic)
                    .map(|(_, sentence)| sentence.clone())
            }),
        })
        .collect();

    let (state, state_note, tone) = if drift.is_empty() {
        (
            "current",
            "every sentence in the recording is still true".to_string(),
            "good",
        )
    } else {
        (
            "out of date",
            format!(
                "{} in the recording {} no longer true",
                say::count(drift.len() as u64, "sentence", "sentences"),
                if drift.len() == 1 { "is" } else { "are" }
            ),
            "watch",
        )
    };

    Ok(Some(Tour {
        video,
        script,
        chapters,
        state: state.to_string(),
        state_note,
        tone: tone.to_string(),
        drift,
        regenerate_command: format!("brain docs generate . --prefix {prefix}"),
    }))
}

/// The narration is a projection, so the twin records its hash but not its
/// text. Read it from the workspace when it is there — through the same
/// containment check every other body goes through.
fn read_narration(content_root: Option<&std::path::Path>) -> Option<String> {
    let root = content_root?;
    let path = crate::body::safe_content_path(root, "docs/generated/narration.txt").ok()?;
    std::fs::read_to_string(path).ok()
}
