//! Before: the pre-edit briefing.
//!
//! The question an agent asks before every edit is always the same: what
//! depends on this, which tests cover it, which documents constrain it,
//! how busy has it been, what did past sessions learn here — and am I
//! even allowed to write it? This module composes the answers from
//! queries that already exist into one budgeted response. Nothing here is
//! stored; like wake, it is data first ([`Briefing`]) and text second
//! ([`render`]).

use crate::assoc::AssocIndex;
use crate::kinds;
use crate::twin::{latest, latest_at, live_to, notes as entity_notes, sid_label};
use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::{Index, MemIndex};
use brain_store::{now_ms, Store, StoreError};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fmt::Write as _;

/// Everything `brain before` says, uncapped. Rendering truncates; the
/// data never does.
#[derive(Serialize)]
pub struct Briefing {
    pub name: String,
    pub prefix: String,
    pub write: WriteAccess,
    pub blast: Blast,
    /// Test files whose `covers` relation reaches this file.
    pub covered_by: Vec<String>,
    /// Documents (decisions, plans, ...) that mention this file.
    pub mentioned_by: Vec<DocRef>,
    pub churn: Churn,
    /// Notes on this entity, newest first.
    pub notes: Vec<NoteRow>,
    /// Strongest associations, with reasons.
    pub related: Vec<RelatedRow>,
}

/// Whether the file may be edited directly — the projection contract on
/// the file entity is the authoritative signal (ADR-019).
#[derive(Serialize, PartialEq, Debug)]
#[serde(tag = "access", rename_all = "snake_case")]
pub enum WriteAccess {
    /// A plain file: edit it; the twin observes the change on refresh.
    File,
    /// A captured file-first kind: edit the file; the twin records it as
    /// this kind on refresh.
    Captured { kind: String },
    /// A read-only projection: the graph owns it. Never edit the file.
    Projection {
        kind: String,
        slug: String,
        edit_via: String,
    },
}

#[derive(Serialize)]
pub struct Blast {
    /// Files that import this one, directly.
    pub direct: Vec<String>,
    /// Distinct files reached by following importers transitively.
    pub transitive: usize,
}

#[derive(Serialize, Debug)]
pub struct DocRef {
    pub slug: String,
    pub kind: String,
}

#[derive(Serialize)]
pub struct Churn {
    /// Content versions the twin has observed.
    pub versions: usize,
    pub last_observed_ms: Option<u64>,
}

#[derive(Serialize)]
pub struct NoteRow {
    pub at_ms: u64,
    pub text: String,
}

#[derive(Serialize)]
pub struct RelatedRow {
    pub label: String,
    pub score: u32,
    pub reasons: Vec<String>,
}

/// Decide write access for a path without composing the full briefing —
/// the query form of the authoring gate, shared with `brain can-i`.
pub fn write_access(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    sid: &StableId,
    rel: &str,
) -> Result<WriteAccess, StoreError> {
    if latest(index, store, sid, "generated")?.as_deref() == Some("true") {
        let mut kind = String::new();
        let mut slug = String::new();
        for (_, artifact) in live_to(index, store, sid, "projected_to")? {
            for node in index.entity_nodes(&artifact) {
                if let Ok(Object::Entity {
                    entity_kind,
                    labels,
                    ..
                }) = store.get(&node)
                {
                    slug = labels.get("slug").cloned().unwrap_or_default();
                    kind = entity_kind;
                    break;
                }
            }
        }
        let edit_via = if kind.is_empty() {
            "generated file — repair via `brain tidy`".to_string()
        } else {
            format!("brain artifact edit {prefix} {kind} {slug}")
        };
        return Ok(WriteAccess::Projection {
            kind,
            slug,
            edit_via,
        });
    }
    let registry = kinds::registry(store, index)?;

    // A graph-first kind owns its projection paths even before a file
    // exists there: hand-writing docs/brain/plans/new.md is wrong from
    // the first byte, not once the render lands.
    for def in registry.values() {
        if def.placement != "graph_first" || def.project_to.is_empty() {
            continue;
        }
        if crate::templates::glob_match(&def.project_to.replace("{slug}", "*"), rel) {
            let slug = std::path::Path::new(rel)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let edit_via = format!("brain artifact new|edit {prefix} {} {slug}", def.kind);
            return Ok(WriteAccess::Projection {
                kind: def.kind.clone(),
                slug,
                edit_via,
            });
        }
    }

    // File-first kinds: any capture or home glob claims the path. The
    // most literal pattern wins, matching the twin's own routing.
    let mut best: Option<(usize, String)> = None;
    for def in registry.values() {
        for pattern in def.capture.iter().chain(def.home.iter()) {
            if !crate::templates::glob_match(pattern, rel) {
                continue;
            }
            let literal = pattern.chars().filter(|c| !matches!(c, '*' | '?')).count();
            let better = match &best {
                None => true,
                Some((l, k)) => literal > *l || (literal == *l && def.kind < *k),
            };
            if better {
                best = Some((literal, def.kind.clone()));
            }
        }
    }
    if let Some((_, kind)) = best {
        return Ok(WriteAccess::Captured { kind });
    }
    // Decisions and plans are also routed by path convention (any
    // `/adr/` or `/plans/` directory), not only by registry globs.
    if let Some((kind, _)) = crate::docs::path_kind(rel) {
        return Ok(WriteAccess::Captured {
            kind: kind.as_str().to_string(),
        });
    }
    Ok(WriteAccess::File)
}

/// Compose the pre-edit briefing for a bound file under `prefix`.
pub fn before(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    name: &str,
    sid: &StableId,
) -> Result<Briefing, StoreError> {
    let rel = name
        .strip_prefix(&format!("{prefix}/"))
        .unwrap_or(name)
        .to_string();

    let write = write_access(store, index, prefix, sid, &rel)?;

    // Blast radius: direct importers, then the transitive closure.
    let mut direct: Vec<String> = Vec::new();
    for (_, importer) in live_to(index, store, sid, "imports")? {
        let label = sid_label(index, store, &importer);
        if !direct.contains(&label) {
            direct.push(label);
        }
    }
    let mut seen: BTreeSet<StableId> = BTreeSet::new();
    let mut frontier = vec![sid.clone()];
    for _ in 0..64 {
        if frontier.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for cur in frontier {
            for (_, importer) in live_to(index, store, &cur, "imports")? {
                if importer != *sid && seen.insert(importer.clone()) {
                    next.push(importer);
                }
            }
        }
        frontier = next;
    }
    let blast = Blast {
        direct,
        transitive: seen.len(),
    };

    let covered_by: Vec<String> = live_to(index, store, sid, "covers")?
        .iter()
        .map(|(_, t)| sid_label(index, store, t))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let mut mentioned_by: Vec<DocRef> = Vec::new();
    for (_, from) in live_to(index, store, sid, "mentions")? {
        for node in index.entity_nodes(&from) {
            if let Ok(Object::Entity {
                entity_kind,
                labels,
                ..
            }) = store.get(&node)
            {
                let slug = labels
                    .get("slug")
                    .cloned()
                    .unwrap_or_else(|| sid_label(index, store, &from));
                if !mentioned_by
                    .iter()
                    .any(|d| d.slug == slug && d.kind == entity_kind)
                {
                    mentioned_by.push(DocRef {
                        slug,
                        kind: entity_kind,
                    });
                }
                break;
            }
        }
    }

    let mut versions = 0;
    for node in index.observations_of(sid) {
        if let Ok(Object::Observation { property, .. }) = store.get(&node) {
            if property == "content_b3" {
                versions += 1;
            }
        }
    }
    let churn = Churn {
        versions,
        last_observed_ms: latest_at(index, store, sid, "content_b3")?.map(|(at, _)| at),
    };

    let notes = entity_notes(index, store, sid)?
        .into_iter()
        .rev()
        .map(|(at_ms, text)| NoteRow { at_ms, text })
        .collect();

    let related = AssocIndex::build(store, index, prefix)?
        .related(sid)
        .into_iter()
        .map(|(label, score, reasons)| RelatedRow {
            label,
            score,
            reasons,
        })
        .collect();

    Ok(Briefing {
        name: name.to_string(),
        prefix: prefix.to_string(),
        write,
        blast,
        covered_by,
        mentioned_by,
        churn,
        notes,
        related,
    })
}

/// The textual projection of a [`Briefing`]: one screen, ranked, capped —
/// every truncation says so.
pub fn render(b: &Briefing) -> String {
    let now = now_ms();
    let mut out = String::new();
    writeln!(out, "== before: {} ==", b.name).ok();

    match &b.write {
        WriteAccess::File => {
            writeln!(
                out,
                "write: ok — a plain file; the twin observes changes on refresh"
            )
            .ok();
        }
        WriteAccess::Captured { kind } => {
            writeln!(
                out,
                "write: ok — captured as {kind} on refresh (file-first)"
            )
            .ok();
        }
        WriteAccess::Projection {
            kind,
            slug,
            edit_via,
        } => {
            writeln!(
                out,
                "write: REFUSED — read-only projection of {kind}/{slug}; edit via `{edit_via}`"
            )
            .ok();
        }
    }

    writeln!(
        out,
        "blast radius: {} direct importer(s), {} transitive",
        b.blast.direct.len(),
        b.blast.transitive
    )
    .ok();
    if !b.blast.direct.is_empty() {
        let shown: Vec<&str> = b.blast.direct.iter().take(3).map(String::as_str).collect();
        let more = b.blast.direct.len().saturating_sub(shown.len());
        let suffix = if more > 0 {
            format!(" (+{more} more)")
        } else {
            String::new()
        };
        writeln!(out, "  {}{suffix}", shown.join(", ")).ok();
    }

    if b.covered_by.is_empty() {
        writeln!(out, "tests: nothing observed covers this file").ok();
    } else {
        writeln!(out, "tests: covered by {}", b.covered_by.join(", ")).ok();
    }

    if !b.mentioned_by.is_empty() {
        let shown: Vec<String> = b
            .mentioned_by
            .iter()
            .take(4)
            .map(|d| format!("{} ({})", d.slug, d.kind))
            .collect();
        let more = b.mentioned_by.len().saturating_sub(shown.len());
        let suffix = if more > 0 {
            format!(" (+{more} more)")
        } else {
            String::new()
        };
        writeln!(out, "mentioned by: {}{suffix}", shown.join(", ")).ok();
    }

    write!(out, "churn: {} version(s)", b.churn.versions).ok();
    if let Some(at) = b.churn.last_observed_ms {
        write!(out, ", last observed {}", age(now, at)).ok();
    }
    writeln!(out).ok();

    if !b.notes.is_empty() {
        writeln!(out, "notes ({}, newest first):", b.notes.len()).ok();
        for n in b.notes.iter().take(3) {
            writeln!(out, "  [{}] {}", age(now, n.at_ms), clip(&n.text, 140)).ok();
        }
        if b.notes.len() > 3 {
            writeln!(out, "  … {} more — brain notes {}", b.notes.len() - 3, b.name).ok();
        }
    }

    if !b.related.is_empty() {
        writeln!(out, "related:").ok();
        for r in b.related.iter().take(3) {
            writeln!(out, "  [{:>3}] {} — {}", r.score, r.label, r.reasons.join(", ")).ok();
        }
    }

    write!(
        out,
        "next: brain twin rdeps {n} --transitive | brain notes {n} | brain related {n}",
        n = b.name
    )
    .ok();
    out
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{cut}…")
}

fn age(now: u64, at: u64) -> String {
    let s = now.saturating_sub(at) / 1000;
    if s >= 86_400 {
        format!("{}d ago", s / 86_400)
    } else if s >= 3_600 {
        format!("{}h ago", s / 3_600)
    } else if s >= 60 {
        format!("{}m ago", s / 60)
    } else {
        format!("{s}s ago")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::twin::refresh;
    use brain_index::replay;
    use std::fs;

    #[test]
    fn briefing_composes_the_pre_edit_answer() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::create_dir_all(src.path().join("docs/plans")).unwrap();
        fs::write(src.path().join("src/util.rs"), "pub fn helper() {}\n").unwrap();
        fs::write(
            src.path().join("src/main.rs"),
            "use crate::util::helper;\npub fn main() { helper() }\n",
        )
        .unwrap();
        fs::write(
            src.path().join("docs/plans/build-x.md"),
            "# Build X\n\nRework src/util.rs.\n",
        )
        .unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = brain_store::Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();

        let sid = StableId::derive(&["file", "src/util.rs"]);
        let b = before(&store, &index, "twin/app", "twin/app/src/util.rs", &sid).unwrap();
        assert_eq!(b.write, WriteAccess::File);
        assert_eq!(b.blast.direct, vec!["src/main.rs"]);
        assert_eq!(b.blast.transitive, 1);
        assert!(
            b.mentioned_by
                .iter()
                .any(|d| d.slug == "build-x" && d.kind == "plan"),
            "plan mention surfaces: {:?}",
            b.mentioned_by
        );
        assert_eq!(b.churn.versions, 1);

        let text = render(&b);
        assert!(text.contains("write: ok — a plain file"), "{text}");
        assert!(text.contains("blast radius: 1 direct importer(s), 1 transitive"), "{text}");
        assert!(text.contains("build-x (plan)"), "{text}");

        // A captured file-first kind names itself.
        let plan_sid = StableId::derive(&["file", "docs/plans/build-x.md"]);
        let b = before(
            &store,
            &index,
            "twin/app",
            "twin/app/docs/plans/build-x.md",
            &plan_sid,
        )
        .unwrap();
        assert_eq!(
            b.write,
            WriteAccess::Captured {
                kind: "plan".to_string()
            }
        );

        // The same briefing, as data.
        let v = serde_json::to_value(&b).unwrap();
        assert_eq!(v["write"]["access"], "captured");
        assert_eq!(v["write"]["kind"], "plan");
    }
}
