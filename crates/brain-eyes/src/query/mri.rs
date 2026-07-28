//! The living graph: every entity, laid out once, in three dimensions.
//!
//! The first attempt at this put 1,297 nodes on a golden-angle spiral that
//! ignored edges entirely and then dropped three quarters of them in the
//! browser. ADR-024 removed it. This is the rebuild, and the difference is
//! not the dimension count — it is where the layout happens and what the
//! geometry means.
//!
//! **Layout is computed here, once per graph version, and cached.** The
//! browser orbits a fixed anatomy; it never runs a force simulation. That
//! is what makes "stable anatomy, moving activity" possible: nothing
//! drifts while you are reading it, so any motion on screen is a fact.
//!
//! **Position carries three claims.** Height is dependency depth — what
//! everything rests on sits at the bottom. Clusters are the categories a
//! developer already thinks in, and they never move. Inside a cluster,
//! things that reference each other are pulled together, so a module looks
//! like a module.
//!
//! **Detail resolves; it is never dropped.** Every node is in the payload
//! with a level: landmarks (features, decisions, runs), ordinary things
//! (files, tests, documents), and fine detail (functions and types). The
//! camera decides what to draw, and the view says how much is on screen.

use crate::dto::*;
use crate::query;
use crate::say;
use crate::state::{EventPayload, Loaded};
use brain_core::ids::StableId;
use brain_observe::twin;
use std::collections::{BTreeMap, BTreeSet};

/// Clusters, in ring order. Stable: this list is the anatomy.
const CLUSTERS: &[(&str, &str, &str, &[&str])] = &[
    (
        "implementation",
        "Implementation",
        "The code itself, stacked by what depends on what.",
        &["source_file", "symbol"],
    ),
    (
        "features",
        "Features",
        "What the system claims to do.",
        &["feature"],
    ),
    (
        "tests",
        "Tests",
        "Cases and the runs that graded them.",
        &["test_case", "test_run"],
    ),
    (
        "decisions",
        "Decisions",
        "Why the system is the way it is.",
        &["decision"],
    ),
    (
        "documentation",
        "Documentation",
        "Prose that is meant to track the code.",
        &["doc", "runbook", "plan", "task_list", "skill", "agent_config"],
    ),
    (
        "artifacts",
        "Artifacts",
        "Pictures, recordings and contracts.",
        &["asset", "prototype", "capability_matrix", "template"],
    ),
    (
        "work",
        "Work",
        "Agent sessions and governed changes.",
        &["agent_session", "change"],
    ),
    (
        "outside",
        "Outside",
        "Dependencies this workspace does not contain.",
        &["module"],
    ),
];

/// How far out the satellite clusters orbit the code.
const RING: f32 = 470.0;
/// Vertical distance between dependency layers. Large enough that the
/// stack is the first thing you notice about the shape.
const LAYER_HEIGHT: f32 = 72.0;
/// Spacing between module centres. A module has to be further from its
/// neighbour than it is wide, or the code reads as one cloud.
const MODULE_PITCH: f32 = 44.0;
/// Force-relaxation passes inside a group.
const PASSES: usize = 90;

pub fn build(loaded: &Loaded) -> Result<MriView, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let prefix = loaded.prefix();
    let now = loaded.snapshot.generated_at_ms;

    // ---- gather -----------------------------------------------------
    let mut nodes: Vec<MriNode> = Vec::new();
    let mut position: BTreeMap<StableId, u32> = BTreeMap::new();
    let mut ids: Vec<StableId> = Vec::new();

    let layers = module_layers(loaded)?;
    let recent = recently_changed(loaded);
    let failing: BTreeSet<StableId> = query::scoped(index, store, prefix, "test_case")?
        .into_iter()
        .filter(|(sid, _)| {
            twin::latest(index, store, sid, "result").ok().flatten().as_deref() == Some("fail")
        })
        .map(|(sid, _)| sid)
        .collect();

    // Files and the symbols they declare are scoped by the namespace, not
    // by a prefix label — they carry only their path — so they are
    // gathered through the same helper every other file query uses.
    let mut implementation: Vec<(StableId, BTreeMap<String, String>, &str, String)> = Vec::new();
    for (path, sid) in query::present_files(index, store, prefix)? {
        let module = super::map::module_of(&path);
        for (_, symbol) in twin::live_from(index, store, &sid, "contains")
            .map_err(|e| e.to_string())?
        {
            let labels = query::labels_of(index, store, &symbol);
            implementation.push((symbol, labels, "symbol", module.clone()));
        }
        let mut labels = BTreeMap::new();
        labels.insert("path".to_string(), path);
        implementation.push((sid, labels, "source_file", module));
    }

    for (cluster, _, _, kinds) in CLUSTERS {
        for kind in *kinds {
            let found: Vec<(StableId, BTreeMap<String, String>, &str, String)> =
                if *cluster == "implementation" {
                    implementation
                        .iter()
                        .filter(|(_, _, entity_kind, _)| entity_kind == kind)
                        .cloned()
                        .collect()
                } else {
                    query::scoped(index, store, prefix, kind)?
                        .into_iter()
                        .map(|(sid, labels)| (sid, labels, *kind, cluster.to_string()))
                        .collect()
                };
            for (sid, labels, _, group) in found {
                if position.contains_key(&sid) {
                    continue;
                }
                let level = match *kind {
                    "feature" | "decision" | "test_run" | "change" | "agent_session" => 0,
                    "symbol" => 2,
                    _ => 1,
                };
                let pulse = if failing.contains(&sid) {
                    Some("failing".to_string())
                } else if *kind == "agent_session"
                    && twin::latest(index, store, &sid, "ended_at")
                        .ok()
                        .flatten()
                        .and_then(|v| v.parse::<u64>().ok())
                        .is_some_and(|end| now.saturating_sub(end) < 20 * 60 * 1000)
                {
                    Some("working".to_string())
                } else if *kind == "change"
                    && matches!(
                        twin::latest(index, store, &sid, "status")
                            .ok()
                            .flatten()
                            .as_deref(),
                        Some("proposed") | Some("applied") | Some("indeterminate")
                    )
                {
                    Some("unfinished".to_string())
                } else if recent.contains(&sid) {
                    Some("changed".to_string())
                } else {
                    None
                };

                position.insert(sid.clone(), nodes.len() as u32);
                ids.push(sid.clone());
                nodes.push(MriNode {
                    label: query::display_name(index, store, &sid, &labels),
                    id: sid.to_string(),
                    kind: kind.to_string(),
                    glyph: say::kind_glyph(kind).to_string(),
                    cluster: cluster.to_string(),
                    group,
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    size: match *kind {
                        "feature" => 3.4,
                        "decision" | "test_run" | "change" | "agent_session" => 2.6,
                        "symbol" => 1.0,
                        _ => 1.8,
                    },
                    level,
                    tone: "quiet".to_string(),
                    pulse,
                    at_ms: 0,
                });
            }
        }
    }

    // ---- edges ------------------------------------------------------
    let mut edges: Vec<MriEdge> = Vec::new();
    let mut seen: BTreeSet<(u32, u32, String)> = BTreeSet::new();
    for sid in &ids {
        let Some(&a) = position.get(sid) else { continue };
        for predicate in [
            "imports",
            "contains",
            "covers",
            "mentions",
            "defined_in",
            "touched",
            "depicts",
            "recorded_in",
            "supersedes",
            "verified_by",
            "implemented_by",
            "tested_by",
            "decided_by",
            "documented_in",
            "changes",
            "attached_to",
            "part_of",
        ] {
            for (_, to) in twin::live_from(index, store, sid, predicate)
                .map_err(|e| e.to_string())?
            {
                let Some(&b) = position.get(&to) else { continue };
                if a == b || !seen.insert((a.min(b), a.max(b), predicate.to_string())) {
                    continue;
                }
                edges.push(MriEdge {
                    a,
                    b,
                    level: nodes[a as usize].level.max(nodes[b as usize].level),
                    predicate: predicate.to_string(),
                });
            }
        }
    }

    // ---- layout -----------------------------------------------------
    let clusters = place(&mut nodes, &edges, &layers);

    let levels = (0..3)
        .map(|level| nodes.iter().filter(|n| n.level == level).count())
        .collect();
    let lit = nodes.iter().filter(|n| n.pulse.is_some()).count();
    let headline = if lit == 0 {
        format!(
            "{} things, still.",
            say::count(nodes.len() as u64, "thing", "things")
        )
    } else {
        format!(
            "{} things, {} of them moving.",
            nodes.len(),
            lit
        )
    };

    Ok(MriView {
        snapshot: loaded.snapshot.clone(),
        headline,
        clusters,
        nodes,
        edges,
        levels,
    })
}

/// Place every node, and return where the clusters ended up.
fn place(
    nodes: &mut [MriNode],
    edges: &[MriEdge],
    layers: &BTreeMap<String, usize>,
) -> Vec<MriCluster> {
    // Groups, in a stable order.
    let mut groups: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (position, node) in nodes.iter().enumerate() {
        groups
            .entry((node.cluster.clone(), node.group.clone()))
            .or_default()
            .push(position);
    }

    let cluster_angle: BTreeMap<&str, f32> = CLUSTERS
        .iter()
        .enumerate()
        .map(|(position, (id, _, _, _))| {
            (
                *id,
                position as f32 / CLUSTERS.len() as f32 * std::f32::consts::TAU,
            )
        })
        .collect();

    // Implementation is the trunk: it sits at the centre, and the other
    // clusters orbit it. Everything else would put the code in a corner.
    let mut centres: BTreeMap<(String, String), (f32, f32, f32)> = BTreeMap::new();
    let implementation: Vec<&(String, String)> = groups
        .keys()
        .filter(|(cluster, _)| cluster == "implementation")
        .collect();
    for (position, key) in implementation.iter().enumerate() {
        let layer = layers.get(&key.1).copied().unwrap_or(0);
        // A phyllotaxis disc spaces modules evenly without a grid's
        // artificial rows; height is the dependency layer, which is the
        // part that means something.
        let angle = position as f32 * 2.399_963;
        let radius = MODULE_PITCH * (position as f32 + 0.7).sqrt();
        centres.insert(
            (*key).clone(),
            (
                angle.cos() * radius,
                layer as f32 * LAYER_HEIGHT,
                angle.sin() * radius,
            ),
        );
    }
    let peak = layers.values().copied().max().unwrap_or(0) as f32 * LAYER_HEIGHT;
    for key in groups.keys() {
        if key.0 == "implementation" {
            continue;
        }
        let angle = cluster_angle.get(key.0.as_str()).copied().unwrap_or(0.0);
        centres.insert(
            key.clone(),
            (angle.cos() * RING, peak * 0.55, angle.sin() * RING),
        );
    }

    // Intra-group edges only: a group's internal shape is its own.
    let mut internal: BTreeMap<(String, String), Vec<(usize, usize)>> = BTreeMap::new();
    for edge in edges {
        let (a, b) = (edge.a as usize, edge.b as usize);
        let ka = (nodes[a].cluster.clone(), nodes[a].group.clone());
        let kb = (nodes[b].cluster.clone(), nodes[b].group.clone());
        if ka == kb {
            internal.entry(ka).or_default().push((a, b));
        }
    }

    for (key, members) in &groups {
        let centre = centres.get(key).copied().unwrap_or((0.0, 0.0, 0.0));
        let links = internal.get(key).cloned().unwrap_or_default();
        relax(nodes, members, &links, centre);
    }

    // Cluster summaries, for the legend and the far zoom.
    let mut out = Vec::new();
    for (id, label, note, _) in CLUSTERS {
        let members: Vec<&MriNode> = nodes.iter().filter(|n| n.cluster == *id).collect();
        if members.is_empty() {
            continue; // absence is silence, here too
        }
        let count = members.len();
        let (mut x, mut y, mut z) = (0.0, 0.0, 0.0);
        for node in &members {
            x += node.x;
            y += node.y;
            z += node.z;
        }
        let (x, y, z) = (
            x / count as f32,
            y / count as f32,
            z / count as f32,
        );
        let radius = members
            .iter()
            .map(|n| {
                let (dx, dy, dz) = (n.x - x, n.y - y, n.z - z);
                (dx * dx + dy * dy + dz * dz).sqrt()
            })
            .fold(1.0f32, f32::max);
        out.push(MriCluster {
            id: id.to_string(),
            label: label.to_string(),
            note: note.to_string(),
            count,
            x,
            y,
            z,
            radius,
        });
    }
    out
}

/// Settle one group around its centre: linked things attract, everything
/// repels, and the starting positions come from the identifiers so the
/// same graph always produces the same picture.
fn relax(
    nodes: &mut [MriNode],
    members: &[usize],
    links: &[(usize, usize)],
    centre: (f32, f32, f32),
) {
    let n = members.len();
    if n == 0 {
        return;
    }
    let mut slot: BTreeMap<usize, usize> = BTreeMap::new();
    let mut point: Vec<(f32, f32, f32)> = Vec::with_capacity(n);
    let spread = 6.0 * (n as f32).sqrt();
    for (position, &node) in members.iter().enumerate() {
        slot.insert(node, position);
        // Deterministic scatter: the identifier is the seed.
        let seed = fnv(&nodes[node].id);
        let unit = |shift: u32| (((seed >> shift) & 0xffff) as f32 / 65535.0) - 0.5;
        point.push((
            unit(0) * spread,
            unit(16) * spread * 0.45,
            unit(32) * spread,
        ));
    }
    if n == 1 {
        let node = members[0];
        nodes[node].x = centre.0;
        nodes[node].y = centre.1;
        nodes[node].z = centre.2;
        return;
    }

    // Repulsion is capped to near neighbours by a simple distance cutoff,
    // which keeps a 600-file module from costing 360,000 comparisons a
    // pass without changing the result anyone can see.
    let cutoff = spread * 0.6;
    for _ in 0..PASSES {
        let mut force = vec![(0.0f32, 0.0f32, 0.0f32); n];
        for i in 0..n {
            for j in (i + 1)..n {
                let (dx, dy, dz) = (
                    point[i].0 - point[j].0,
                    point[i].1 - point[j].1,
                    point[i].2 - point[j].2,
                );
                let square = dx * dx + dy * dy + dz * dz + 0.01;
                if square > cutoff * cutoff {
                    continue;
                }
                let push = 24.0 / square;
                let distance = square.sqrt();
                let (ux, uy, uz) = (dx / distance, dy / distance, dz / distance);
                force[i].0 += ux * push;
                force[i].1 += uy * push;
                force[i].2 += uz * push;
                force[j].0 -= ux * push;
                force[j].1 -= uy * push;
                force[j].2 -= uz * push;
            }
        }
        for (a, b) in links {
            let (Some(&i), Some(&j)) = (slot.get(a), slot.get(b)) else {
                continue;
            };
            let (dx, dy, dz) = (
                point[j].0 - point[i].0,
                point[j].1 - point[i].1,
                point[j].2 - point[i].2,
            );
            let pull = 0.02;
            force[i].0 += dx * pull;
            force[i].1 += dy * pull;
            force[i].2 += dz * pull;
            force[j].0 -= dx * pull;
            force[j].1 -= dy * pull;
            force[j].2 -= dz * pull;
        }
        for i in 0..n {
            // Gravity toward the group's own centre keeps it compact.
            point[i].0 += force[i].0 * 0.08 - point[i].0 * 0.012;
            point[i].1 += force[i].1 * 0.08 - point[i].1 * 0.030;
            point[i].2 += force[i].2 * 0.08 - point[i].2 * 0.012;
        }
    }

    for (position, &node) in members.iter().enumerate() {
        nodes[node].x = centre.0 + point[position].0;
        nodes[node].y = centre.1 + point[position].1;
        nodes[node].z = centre.2 + point[position].2;
    }
}

/// Dependency depth per module, reused from the Map so the two drawings
/// agree about what rests on what.
fn module_layers(loaded: &Loaded) -> Result<BTreeMap<String, usize>, String> {
    let map = super::map::build(loaded, "attention")?;
    Ok(map
        .blocks
        .into_iter()
        .map(|block| (block.path, block.layer))
        .collect())
}

/// Entities whose content changed since the last consolidation.
///
/// The sleep watermark is the right window: it is what "since you were
/// last here" means everywhere else in Eyes.
fn recently_changed(loaded: &Loaded) -> BTreeSet<StableId> {
    let repo = StableId::derive(&["repo", loaded.prefix()]);
    let since = twin::latest(&loaded.index, &loaded.store, &repo, "consolidated_until")
        .ok()
        .flatten()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_else(|| loaded.snapshot.generated_at_ms.saturating_sub(86_400_000));
    loaded
        .events()
        .iter()
        .filter(|row| row.at_ms > since)
        .filter(|row| {
            matches!(&row.payload,
                EventPayload::Observation { property, .. }
                    if property == "content_b3" || property == "content")
        })
        .filter_map(|row| row.subject.clone())
        .collect()
}

fn fnv(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
