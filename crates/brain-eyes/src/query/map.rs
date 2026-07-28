//! "What is this system made of, and where is the risk?"
//!
//! Not a picture of the graph — a picture of the *system*. Files roll up
//! into the modules a developer already thinks in (a crate, a package, a
//! top-level directory), sized by what they hold, stacked by which depends
//! on which, and coloured by one question at a time. Twenty blocks a
//! person can read beat a thousand dots nobody can.

use crate::dto::*;
use crate::query;
use crate::say;
use crate::state::Loaded;
use brain_core::ids::StableId;
use brain_observe::twin;
use std::collections::{BTreeMap, BTreeSet};

const LENSES: &[(&str, &str, &str)] = &[
    (
        "attention",
        "Where the pressure is",
        "Modules holding the files that most deserve a look right now.",
    ),
    (
        "tests",
        "What is covered",
        "How much of each module has a test that touches it.",
    ),
    (
        "change",
        "What moved recently",
        "Modules that changed since the last consolidated session.",
    ),
];

struct Block {
    label: String,
    path: String,
    files: Vec<(String, StableId)>,
    symbols: usize,
    covered: usize,
    recent: usize,
    attention: u32,
    stale_docs: usize,
}

pub fn build(loaded: &Loaded, lens: &str) -> Result<MapView, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let lens = LENSES
        .iter()
        .find(|(id, _, _)| *id == lens)
        .unwrap_or(&LENSES[0]);

    let insights = loaded.insights();
    let ranked = loaded.attention();
    let attention_by_path: BTreeMap<&str, u32> = ranked
        .iter()
        .map(|item| (item.label.as_str(), item.score))
        .collect();

    let repo = StableId::derive(&["repo", prefix]);
    let watermark: u64 = twin::latest(index, store, &repo, "consolidated_until")
        .map_err(|e| e.to_string())?
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);

    // ---- gather files into modules --------------------------------------
    let files = query::present_files(index, store, prefix)?;
    let mut blocks: BTreeMap<String, Block> = BTreeMap::new();
    let mut file_block: BTreeMap<StableId, String> = BTreeMap::new();

    for (path, sid) in &files {
        let key = module_of(path);
        file_block.insert(sid.clone(), key.clone());
        let block = blocks.entry(key.clone()).or_insert_with(|| Block {
            label: module_label(&key),
            path: key.clone(),
            files: Vec::new(),
            symbols: 0,
            covered: 0,
            recent: 0,
            attention: 0,
            stale_docs: 0,
        });
        block.files.push((path.clone(), sid.clone()));
        block.symbols += twin::live_from(index, store, sid, "contains")
            .map_err(|e| e.to_string())?
            .len();
        let declared: usize = twin::latest(index, store, sid, "tests_declared")
            .map_err(|e| e.to_string())?
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let covered = declared > 0
            || !twin::live_to(index, store, sid, "covers")
                .map_err(|e| e.to_string())?
                .is_empty();
        if covered {
            block.covered += 1;
        }
        if let Some((at, _)) =
            twin::latest_at(index, store, sid, "content_b3").map_err(|e| e.to_string())?
        {
            if at > watermark && watermark > 0 {
                block.recent += 1;
            }
        }
        block.attention += attention_by_path.get(path.as_str()).copied().unwrap_or(0);
    }

    // Documents that drifted belong to the module they describe.
    for doc in &insights.stale_docs {
        if doc.severity != twin::Severity::Warn {
            continue;
        }
        for changed in &doc.changed {
            if let Some(key) = blocks.keys().find(|key| changed.starts_with(*key as &str)) {
                let key = key.clone();
                if let Some(block) = blocks.get_mut(&key) {
                    block.stale_docs += 1;
                }
            }
        }
    }

    // ---- module-level dependency edges ----------------------------------
    let mut weights: BTreeMap<(String, String), usize> = BTreeMap::new();
    for (_, sid) in &files {
        let Some(from) = file_block.get(sid) else {
            continue;
        };
        for (_, target) in twin::live_from(index, store, sid, "imports")
            .map_err(|e| e.to_string())?
        {
            let Some(to) = file_block.get(&target) else {
                continue; // an external dependency, not a module of ours
            };
            if to == from {
                continue;
            }
            *weights.entry((from.clone(), to.clone())).or_insert(0) += 1;
        }
    }

    let layers = layer_blocks(&blocks.keys().cloned().collect::<Vec<_>>(), &weights);

    // ---- render ---------------------------------------------------------
    let max_attention = blocks.values().map(|b| b.attention).max().unwrap_or(0).max(1);
    let mut out: Vec<MapBlock> = blocks
        .values()
        .map(|block| {
            let files = block.files.len();
            let (value, sentence, tone) = match lens.0 {
                "tests" => {
                    let percent = if files == 0 {
                        0
                    } else {
                        (block.covered * 100 / files) as u32
                    };
                    let tone = match percent {
                        0..=33 => "bad",
                        34..=79 => "watch",
                        _ => "good",
                    };
                    (
                        percent,
                        format!(
                            "{} of {} have a test that touches them",
                            block.covered,
                            say::count(files as u64, "file", "files")
                        ),
                        tone,
                    )
                }
                "change" => {
                    let percent = if files == 0 {
                        0
                    } else {
                        (block.recent * 100 / files) as u32
                    };
                    let sentence = if watermark == 0 {
                        "no consolidated session yet, so nothing counts as recent".to_string()
                    } else if block.recent == 0 {
                        "untouched since your last session".to_string()
                    } else {
                        format!(
                            "{} changed since your last session",
                            say::count(block.recent as u64, "file", "files")
                        )
                    };
                    (percent, sentence, if percent > 50 { "watch" } else { "quiet" })
                }
                _ => {
                    let percent = (block.attention * 100 / max_attention).min(100);
                    let sentence = if block.attention == 0 {
                        "nothing here is asking for attention".to_string()
                    } else {
                        format!(
                            "{} carrying most of this module's pressure",
                            say::count(
                                block
                                    .files
                                    .iter()
                                    .filter(|(path, _)| attention_by_path.contains_key(path.as_str()))
                                    .count() as u64,
                                "file is",
                                "files are"
                            )
                        )
                    };
                    (percent, sentence, if percent > 60 { "watch" } else { "quiet" })
                }
            };

            let mut facts = vec![format!(
                "{}, {}",
                say::count(files as u64, "file", "files"),
                say::count(block.symbols as u64, "function or type", "functions and types")
            )];
            if block.stale_docs > 0 {
                facts.push(format!(
                    "{} here drifted from a document",
                    say::count(block.stale_docs as u64, "file", "files")
                ));
            }

            MapBlock {
                id: block.path.clone(),
                label: block.label.clone(),
                path: block.path.clone(),
                files,
                symbols: block.symbols,
                layer: layers.get(&block.path).copied().unwrap_or(0),
                value,
                tone: tone.to_string(),
                sentence,
                facts,
            }
        })
        .collect();
    out.sort_by(|a, b| a.layer.cmp(&b.layer).then(b.files.cmp(&a.files)));
    out.truncate(40);

    let kept: BTreeSet<String> = out.iter().map(|block| block.id.clone()).collect();
    let edges: Vec<MapEdge> = weights
        .into_iter()
        .filter(|((from, to), _)| kept.contains(from) && kept.contains(to))
        .map(|((from, to), weight)| MapEdge { from, to, weight })
        .collect();

    let sentence = format!(
        "{} in {}. Blocks lower down are depended on by the ones above.",
        say::count(files.len() as u64, "file", "files"),
        say::count(out.len() as u64, "module", "modules")
    );

    Ok(MapView {
        snapshot: loaded.snapshot.clone(),
        lens: lens.0.to_string(),
        lens_label: lens.1.to_string(),
        lens_note: lens.2.to_string(),
        lenses: LENSES
            .iter()
            .map(|(id, label, _)| (id.to_string(), label.to_string()))
            .collect(),
        blocks: out,
        edges,
        sentence,
    })
}

/// The module a path belongs to: a workspace member if there is one, else
/// its top-level directory.
pub fn module_of(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        [container, member, ..]
            if matches!(*container, "crates" | "packages" | "apps" | "libs" | "services") =>
        {
            format!("{container}/{member}")
        }
        [single] => {
            let _ = single;
            "(top level)".to_string()
        }
        [first, ..] => (*first).to_string(),
        [] => "(top level)".to_string(),
    }
}

fn module_label(key: &str) -> String {
    key.rsplit('/').next().unwrap_or(key).to_string()
}

/// Depth in the dependency order: a module that depends on nothing sits at
/// the bottom. Cycles settle rather than spin.
fn layer_blocks(
    keys: &[String],
    weights: &BTreeMap<(String, String), usize>,
) -> BTreeMap<String, usize> {
    let mut layers: BTreeMap<String, usize> = keys.iter().map(|key| (key.clone(), 0)).collect();
    for _ in 0..8 {
        let mut changed = false;
        for ((from, to), _) in weights {
            let target = layers.get(to).copied().unwrap_or(0);
            let current = layers.get(from).copied().unwrap_or(0);
            if current < target + 1 {
                layers.insert(from.clone(), target + 1);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    layers
}
