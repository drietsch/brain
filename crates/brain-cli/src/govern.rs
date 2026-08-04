//! Governed changes: propose, apply, verify — and the recovery path.

use brain_core::object::Object;
use brain_index::Index;
use std::collections::BTreeSet;
use crate::support::*;

pub(crate) fn cmd_recover() -> Result<(), String> {
    let store = open_store()?;
    let marked = store.intents().recover().map_err(|e| e.to_string())?;
    if marked.is_empty() {
        println!("no pending intents; nothing to recover");
    } else {
        for intent in &marked {
            println!("indeterminate: {intent}");
        }
        println!(
            "{} intent(s) marked indeterminate — NOT retried; reconcile before re-attempting",
            marked.len()
        );
    }
    // Governed changes whose intents just went indeterminate get marked too.
    for (name, node) in store.namespace().map_err(|e| e.to_string())? {
        if let Ok(Object::Entity { entity_kind, .. }) = store.get(&node) {
            if entity_kind == "repo" {
                for slug in
                    brain_observe::govern::reconcile(&store, &name).map_err(|e| e.to_string())?
                {
                    println!("change '{slug}' ({name}) marked indeterminate");
                }
            }
        }
    }
    Ok(())
}

/// `brain change ...` — governed mode: the motor system. Changes to
/// twinned software go through the intent/receipt boundary, with an
/// explicit capability — never ambient authority.
pub(crate) fn cmd_change(args: &[String]) -> Result<(), String> {
    use brain_observe::govern;
    let usage = "usage: brain change propose <prefix> <path> --from <file> [--reason R] [--dir d] | \
                 apply|revert <prefix> <slug> --cap fs [--dir d] | verify <prefix> <slug> [--dir d] | \
                 list <prefix> | show <prefix> <slug>";
    let flag = |key: &str| -> Option<String> {
        args.iter()
            .position(|a| a == key)
            .and_then(|i| args.get(i + 1).cloned())
    };
    let dir = flag("--dir").unwrap_or_else(|| ".".to_string());
    let root = std::path::Path::new(&dir);
    match args.first().map(String::as_str) {
        Some("propose") => {
            let (prefix, path) = match (args.get(1), args.get(2)) {
                (Some(p), Some(t)) if !p.starts_with("--") && !t.starts_with("--") => (p, t),
                _ => return Err(usage.to_string()),
            };
            let from = flag("--from").ok_or("propose needs --from <content-file>")?;
            let reason = flag("--reason").unwrap_or_else(|| "unstated".to_string());
            let content =
                std::fs::read_to_string(&from).map_err(|e| format!("cannot read '{from}': {e}"))?;
            let store = open_store()?;
            let p = govern::propose(&store, root, prefix, path, &content, &reason)
                .map_err(|e| e.to_string())?;
            let state = if p.wrote {
                "proposed"
            } else {
                "already proposed"
            };
            println!(
                "change '{}' {state}: {} -> {} ({path})",
                p.slug,
                p.before_b3.as_deref().map(|h| &h[..8]).unwrap_or("absent"),
                &p.after_b3[..8]
            );
            println!(
                "apply with: brain change apply {prefix} {} --cap fs",
                p.slug
            );
            Ok(())
        }
        Some(op @ ("apply" | "revert")) => {
            let (prefix, slug) = match (args.get(1), args.get(2)) {
                (Some(p), Some(s)) if !p.starts_with("--") && !s.starts_with("--") => (p, s),
                _ => return Err(usage.to_string()),
            };
            let caps: Vec<String> = args
                .iter()
                .enumerate()
                .filter(|(_, a)| *a == "--cap")
                .filter_map(|(i, _)| args.get(i + 1).cloned())
                .collect();
            let store = open_store()?;
            let done = if op == "apply" {
                govern::apply(&store, root, prefix, slug, &caps)
            } else {
                govern::revert(&store, root, prefix, slug, &caps)
            }
            .map_err(|e| e.to_string())?;
            println!(
                "{op} {}: intent {} -> receipt {} ({})",
                slug,
                done.intent,
                done.receipt,
                if done.ok { "ok" } else { "FAILED" }
            );
            if done.ok && op == "apply" {
                println!("verify with: brain change verify {prefix} {slug}");
            }
            Ok(())
        }
        Some("verify") => {
            let (prefix, slug) = match (args.get(1), args.get(2)) {
                (Some(p), Some(s)) => (p, s),
                _ => return Err(usage.to_string()),
            };
            let store = open_store()?;
            let v = govern::verify(&store, root, prefix, slug).map_err(|e| e.to_string())?;
            println!(
                "{slug}: {} ({}/{} passed)",
                if v.passed { "VERIFIED" } else { "BROKEN" },
                v.total - v.failed,
                v.total
            );
            Ok(())
        }
        Some("list") => {
            let prefix = args.get(1).ok_or(usage)?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let mut seen = BTreeSet::new();
            let mut any = false;
            for node in index.entities_by_kind("change") {
                let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
                    continue;
                };
                if labels.get("prefix") != Some(prefix) || !seen.insert(id.clone()) {
                    continue;
                }
                let slug = labels.get("slug").cloned().unwrap_or_default();
                let status = brain_observe::twin::latest(&index, &store, &id, "status")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default();
                let target = labels.get("target").cloned().unwrap_or_default();
                let reason = labels.get("title").cloned().unwrap_or_default();
                println!("[{status}] {slug}  {target}  — {reason}");
                any = true;
            }
            if !any {
                println!("no changes under {prefix}");
            }
            Ok(())
        }
        Some("show") => {
            let (prefix, slug) = match (args.get(1), args.get(2)) {
                (Some(p), Some(s)) => (p, s),
                _ => return Err(usage.to_string()),
            };
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = brain_observe::govern::change_sid(prefix, slug);
            for prop in [
                "status",
                "target",
                "reason",
                "before_b3",
                "after_b3",
                "intent",
            ] {
                if let Some(v) = brain_observe::twin::latest(&index, &store, &sid, prop)
                    .map_err(|e| e.to_string())?
                {
                    println!("{prop}: {v}");
                }
            }
            // Status timeline: the change's whole life, oldest first.
            let mut timeline = Vec::new();
            for id in index.observations_of(&sid) {
                if let Object::Observation {
                    property,
                    value,
                    observed_at_ms,
                    ..
                } = store.get(&id).map_err(|e| e.to_string())?
                {
                    if property == "status" {
                        timeline.push((observed_at_ms, value));
                    }
                }
            }
            timeline.sort();
            println!("--- status timeline ---");
            for (at, s) in timeline {
                println!("{at}  {s}");
            }
            if let Some(content) = brain_observe::twin::latest(&index, &store, &sid, "content")
                .map_err(|e| e.to_string())?
            {
                println!("--- proposed content ({} bytes) ---", content.len());
            }
            Ok(())
        }
        _ => Err(usage.to_string()),
    }
}
