//! The twin: refresh it, read what it observed, and record what a person noticed.

use brain_core::object::Object;
use brain_index::Index;
use brain_store::now_ms;
use crate::support::*;

pub(crate) fn print_twin_report(report: &brain_observe::twin::TwinReport, wrote: bool) {
    for f in &report.added {
        println!("added    {f}");
    }
    for f in &report.changed {
        println!("changed  {f}");
    }
    for f in &report.deleted {
        println!("deleted  {f}");
    }
    for f in &report.docs {
        println!("doc      {f}");
    }
    let verb = if wrote { "recorded" } else { "would record" };
    let retracted = if report.retracted > 0 {
        format!(", {} edge(s) retracted", report.retracted)
    } else {
        String::new()
    };
    println!(
        "{} unchanged; {verb} {} added, {} changed, {} deleted ({} symbols, {} relations, {} docs{retracted})",
        report.unchanged,
        report.added.len(),
        report.changed.len(),
        report.deleted.len(),
        report.symbols,
        report.relations,
        report.docs.len()
    );
}

pub(crate) fn cmd_twin_refresh(args: &[String], write: bool) -> Result<(), String> {
    let dir = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("usage: brain twin refresh|status <dir> [--prefix <p>] [--full] [--json]")?;
    let prefix = parse_prefix(&args[1..]);
    let full = args.iter().any(|a| a == "--full");
    let store = open_store()?;
    let path = std::path::Path::new(dir);
    let report = if write && full {
        brain_observe::twin::refresh_full(&store, path, &prefix)
    } else if write {
        brain_observe::twin::refresh(&store, path, &prefix)
    } else {
        brain_observe::twin::status(&store, path, &prefix)
    }
    .map_err(|e| e.to_string())?;
    if wants_json(args) {
        let mut v = serde_json::to_value(&report).map_err(|e| e.to_string())?;
        v["wrote"] = serde_json::Value::Bool(write);
        println!("{v}");
        return Ok(());
    }
    print_twin_report(&report, write);
    Ok(())
}

pub(crate) fn cmd_relation(args: &[String]) -> Result<(), String> {
    let usage = "usage: brain relation retract <from> <predicate> <to> [--prefix <p>]\n       brain relation list <name> [--all] [--prefix <p>]";
    match args.first().map(String::as_str) {
        Some("retract") => {
            let pos = positional(&args[1..]);
            let [from, predicate, to] = pos.as_slice() else {
                return Err(usage.into());
            };
            let prefix = parse_prefix(&args[1..]);
            let store = open_store()?;
            let index = build_index(&store)?;
            let from_sid = resolve_entity(&store, &index, &prefix, from)?;
            let to_sid = resolve_entity(&store, &index, &prefix, to)?;
            if brain_observe::twin::retract(&store, &from_sid, predicate, &to_sid)
                .map_err(|e| e.to_string())?
            {
                println!("retracted {from} -{predicate}-> {to}");
            } else {
                println!("nothing to retract: no live {predicate} edge {from} -> {to}");
            }
            Ok(())
        }
        Some("list") => {
            let name = args.get(1).filter(|a| !a.starts_with("--")).ok_or(usage)?;
            let all = args.iter().any(|a| a == "--all");
            let prefix = parse_prefix(&args[1..]);
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = resolve_entity(&store, &index, &prefix, name)?;
            let mut seen: std::collections::BTreeSet<(String, String, String)> =
                std::collections::BTreeSet::new();
            let mut shown = 0usize;
            for id in store.put_history().map_err(|e| e.to_string())? {
                let Ok(Object::Relation {
                    from,
                    predicate,
                    to,
                    ..
                }) = store.get(&id)
                else {
                    continue;
                };
                let (out, other) = if from == sid {
                    (true, to.clone())
                } else if to == sid {
                    (false, from.clone())
                } else {
                    continue;
                };
                let live = brain_index::edge_active(&*index, &store, &from, &predicate, &to)
                    .map_err(|e| e.to_string())?;
                if !live && !all {
                    continue;
                }
                let label = entity_label(&store, &index, &other);
                let dir = if out { "->" } else { "<-" };
                let flag = if live { "" } else { "  [retracted]" };
                if seen.insert((dir.to_string(), predicate.clone(), label.clone())) {
                    println!("{dir} {predicate:<14} {label}{flag}");
                    shown += 1;
                }
            }
            if shown == 0 {
                println!("no {}relations for {name}", if all { "" } else { "live " });
            }
            Ok(())
        }
        _ => Err(usage.into()),
    }
}

pub(crate) fn cmd_twin(args: &[String]) -> Result<(), String> {
    let usage = "usage: brain twin refresh|status|files|symbols|imports|rdeps|search|insights ...";
    match args.first().map(String::as_str) {
        Some("refresh") => cmd_twin_refresh(&args[1..], true),
        Some("status") => cmd_twin_refresh(&args[1..], false),
        Some("files") => {
            let prefix = args.get(1).ok_or("usage: brain twin files <prefix>")?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let now = now_ms();
            for (name, node) in store.namespace().map_err(|e| e.to_string())? {
                let Some(rel) = name.strip_prefix(&format!("{prefix}/")) else {
                    continue;
                };
                let Ok(Object::Entity { id: sid, .. }) = store.get(&node) else {
                    continue;
                };
                let present = brain_observe::twin::latest(&index, &store, &sid, "present")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_else(|| "true".to_string());
                let age = brain_observe::twin::latest_at(&index, &store, &sid, "content_b3")
                    .map_err(|e| e.to_string())?
                    .map(|(at, _)| format!("{}s", now.saturating_sub(at) / 1000))
                    .unwrap_or_else(|| "?".to_string());
                let symbols = brain_observe::twin::live_from(&index, &store, &sid, "contains")
                    .map_err(|e| e.to_string())?
                    .len();
                let lang = brain_observe::twin::latest(&index, &store, &sid, "language")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default();
                let flag = if present == "false" {
                    let moved = brain_observe::twin::live_from(&index, &store, &sid, "renamed_to")
                        .map_err(|e| e.to_string())?;
                    match moved.first() {
                        Some((_, to)) => {
                            format!("  [moved to {}]", entity_label(&store, &index, to))
                        }
                        None => "  [deleted]".to_string(),
                    }
                } else {
                    String::new()
                };
                println!("{rel}  {lang}  {symbols} symbol(s)  observed {age} ago{flag}");
            }
            Ok(())
        }
        Some("symbols") => {
            let name = args
                .get(1)
                .filter(|a| !a.starts_with("--"))
                .ok_or("usage: brain twin symbols <file-name> [--json]")?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = entity_sid(&store, name)?;
            let mut rows: Vec<(String, String, String)> = Vec::new();
            for target in relation_targets(&store, &index, &sid, "contains")? {
                let line = brain_observe::twin::latest(&index, &store, &target, "line")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_else(|| "?".to_string());
                for node in index.entity_nodes(&target) {
                    if let Ok(Object::Entity { labels, .. }) = store.get(&node) {
                        let kind = labels.get("kind").cloned().unwrap_or_default();
                        let sym = labels.get("name").cloned().unwrap_or_default();
                        rows.push((kind, sym, line.clone()));
                        break;
                    }
                }
            }
            if wants_json(args) {
                let rows: Vec<serde_json::Value> = rows
                    .iter()
                    .map(|(kind, name, line)| {
                        serde_json::json!({"kind": kind, "name": name, "line": line})
                    })
                    .collect();
                println!("{}", serde_json::Value::Array(rows));
                return Ok(());
            }
            for (kind, sym, line) in rows {
                println!("{kind:<10} {sym}  (line {line})");
            }
            Ok(())
        }
        Some(op @ ("imports" | "rdeps")) => {
            let name = args
                .get(1)
                .filter(|a| !a.starts_with("--"))
                .ok_or("usage: brain twin imports|rdeps <file-name> [--transitive] [--json]")?;
            let reverse = op == "rdeps";
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = entity_sid(&store, name)?;
            if args.iter().any(|a| a == "--transitive") {
                // cortex's recursive walk: the full (blast) radius.
                let reached = index
                    .reach(&store, &sid, "imports", reverse, 64)
                    .map_err(|e| e.to_string())?;
                if wants_json(args) {
                    let rows: Vec<serde_json::Value> = reached
                        .iter()
                        .map(|(target, depth)| {
                            serde_json::json!({
                                "label": entity_label(&store, &index, target),
                                "depth": depth,
                            })
                        })
                        .collect();
                    println!("{}", serde_json::Value::Array(rows));
                    return Ok(());
                }
                if reached.is_empty() {
                    println!("nothing, transitively");
                }
                for (target, depth) in reached {
                    println!(
                        "{}{}",
                        "  ".repeat(depth - 1),
                        entity_label(&store, &index, &target)
                    );
                }
                return Ok(());
            }
            let mut targets = Vec::new();
            let live = if reverse {
                brain_observe::twin::live_to(&index, &store, &sid, "imports")
            } else {
                brain_observe::twin::live_from(&index, &store, &sid, "imports")
            }
            .map_err(|e| e.to_string())?;
            for (_, t) in live {
                if !targets.contains(&t) {
                    targets.push(t);
                }
            }
            if wants_json(args) {
                let rows: Vec<String> = targets
                    .iter()
                    .map(|t| entity_label(&store, &index, t))
                    .collect();
                println!("{}", serde_json::to_string(&rows).map_err(|e| e.to_string())?);
                return Ok(());
            }
            if targets.is_empty() {
                println!(
                    "nothing {} {name}",
                    if reverse { "imports" } else { "imported by" }
                );
            }
            for t in targets {
                println!("{}", entity_label(&store, &index, &t));
            }
            Ok(())
        }
        Some("backfill") => {
            let dir = args
                .get(1)
                .filter(|a| !a.starts_with("--"))
                .ok_or("usage: brain twin backfill <dir> [--prefix <p>] [--max-commits N]")?;
            let prefix = parse_prefix(&args[2..]);
            let max = args
                .iter()
                .position(|a| a == "--max-commits")
                .and_then(|i| args.get(i + 1))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let store = open_store()?;
            let report =
                brain_observe::backfill::backfill(&store, std::path::Path::new(dir), &prefix, max)
                    .map_err(|e| e.to_string())?;
            println!(
                "backfilled {} commit(s): {} file version(s), {} deletion(s), {} blob(s) skipped, {} object(s) written",
                report.commits,
                report.file_versions,
                report.deletions,
                report.skipped_blobs,
                report.objects_written
            );
            println!("history is in the graph: churn, `twin at <old-commit>`, and co-change now reach the past");
            Ok(())
        }
        Some("at") => {
            let (prefix, when) = match (args.get(1), args.get(2)) {
                (Some(p), Some(w)) => (p, w),
                _ => {
                    return Err(
                        "usage: brain twin at <prefix> <ms|30m|2h|1d|git-commit|baseline-name>"
                            .into(),
                    )
                }
            };
            let store = open_store()?;
            let index = build_index(&store)?;
            let t = resolve_when(&store, &index, prefix, when)?;
            let files = brain_observe::twin::files_at(&store, &index, prefix, t)
                .map_err(|e| e.to_string())?;
            println!("== {prefix} as of {t} ({} file(s)) ==", files.len());
            for (rel, hash) in files {
                println!("{}  {rel}", &hash[..12]);
            }
            Ok(())
        }
        Some("insights") => {
            let prefix = args
                .get(1)
                .filter(|a| !a.starts_with("--"))
                .ok_or("usage: brain twin insights <prefix> [--json]")?;
            let store = open_store()?;
            let ins = brain_observe::twin::insights(&store, prefix).map_err(|e| e.to_string())?;
            if wants_json(args) {
                println!("{}", serde_json::to_string(&ins).map_err(|e| e.to_string())?);
                return Ok(());
            }
            let now = now_ms();
            println!("== twin insights: {prefix} ==");
            {
                // The last consolidated memory, if the twin has ever slept.
                let index = build_index(&store)?;
                let repo = brain_core::ids::StableId::derive(&["repo", prefix.as_str()]);
                if let Some((at, s)) =
                    brain_observe::twin::latest_at(&index, &store, &repo, "session_summary")
                        .map_err(|e| e.to_string())?
                {
                    let age = now.saturating_sub(at) / 1000;
                    println!("last sleep ({age}s ago): {s}");
                }
            }
            println!(
                "files: {} present, {} deleted   symbols: {}   relations: {}",
                ins.files, ins.deleted_files, ins.symbols, ins.relations
            );
            if let (Some(branch), Some(commit)) = (&ins.git_branch, &ins.git_commit) {
                println!("git: {branch} @ {}", &commit[..commit.len().min(12)]);
            }
            if ins.test_files + ins.tests_declared > 0 {
                let last = ins
                    .last_run
                    .map(|(at, total, passed, failed)| {
                        let age = now.saturating_sub(at) / 1000;
                        let verdict = if failed == 0 { "ok" } else { "FAILED" };
                        format!("; last run {age}s ago: {verdict} ({passed}/{total} passed, {failed} failed)")
                    })
                    .unwrap_or_else(|| "; no runs imported".to_string());
                println!(
                    "tests: {} test file(s), {} declared{last}",
                    ins.test_files, ins.tests_declared
                );
            }
            // Rendering truncates; the data never does. Every shortened
            // list says so — a truncated count must never pose as a total.
            const SHOW: usize = 5;
            let showing = |total: usize, what: &str| {
                if total > SHOW {
                    println!("  … showing {SHOW} of {total} {what}");
                }
            };
            if !ins.failing.is_empty() {
                println!("failing tests ({}):", ins.failing.len());
                for name in ins.failing.iter().take(SHOW) {
                    println!("  ✗ {name}");
                }
                showing(ins.failing.len(), "failing");
            }
            let list = |title: &str, items: &[(String, usize)], unit: &str| {
                if !items.is_empty() {
                    println!("{title}:");
                    for (name, n) in items.iter().take(SHOW) {
                        println!("  {n:>4} {unit}  {name}");
                    }
                    if items.len() > SHOW {
                        println!("  … showing {SHOW} of {}", items.len());
                    }
                }
            };
            if !ins.churn.is_empty() {
                println!("churn (most edited):");
                for (name, n) in ins.churn.iter().take(SHOW) {
                    let tag = if ins.decided.contains(name) {
                        "  [decided]"
                    } else {
                        ""
                    };
                    println!("  {n:>4} versions  {name}{tag}");
                }
                showing(ins.churn.len(), "churned files");
            }
            list("hubs (most imported)", &ins.hubs, "importers");
            list(
                "untested hubs (imported, no tests)",
                &ins.untested_hubs,
                "importers",
            );
            list("largest (symbols declared)", &ins.largest, "symbols");
            list(
                "external deps (unresolved imports)",
                &ins.external_modules,
                "uses",
            );
            if !ins.decisions.is_empty() {
                println!("decisions (ADRs, {} active):", ins.decisions.len());
                for (slug, title, status) in ins.decisions.iter().take(SHOW) {
                    println!("  [{status}] {slug}: {title}");
                }
                showing(ins.decisions.len(), "decisions");
            }
            if !ins.plans.is_empty() {
                println!("plans ({} active):", ins.plans.len());
                for (slug, title) in ins.plans.iter().take(SHOW) {
                    println!("  {slug}: {title}");
                }
                showing(ins.plans.len(), "plans");
            }
            if !ins.skills.is_empty() {
                println!("agent skills:");
                for (slug, agent, desc) in ins.skills.iter().take(SHOW) {
                    println!("  [{agent}] {slug}: {desc}");
                }
                showing(ins.skills.len(), "skills");
            }
            if !ins.agent_configs.is_empty() {
                println!("agent config:");
                for (slug, agent, role) in ins.agent_configs.iter().take(SHOW) {
                    println!("  [{agent}] {slug} ({role})");
                }
                showing(ins.agent_configs.len(), "configs");
            }
            if !ins.custom_artifacts.is_empty() {
                let parts: Vec<String> = ins
                    .custom_artifacts
                    .iter()
                    .map(|(k, n)| format!("{k} ×{n}"))
                    .collect();
                println!(
                    "custom artifacts (graph-defined kinds): {}",
                    parts.join(", ")
                );
            }
            if !ins.features.is_empty() {
                println!("features (progress):");
                for feature in &ins.features {
                    let counted = if feature.by_parts { "parts" } else { "linked" };
                    println!(
                        "  [{}] {}  {} {counted}",
                        feature.status, feature.slug, feature.fraction
                    );
                }
            }
            if !ins.nonconforming.is_empty() {
                println!("nonconforming docs (template contract):");
                for (slug, kind, missing) in &ins.nonconforming {
                    println!("  {slug} ({kind}): missing {missing}");
                }
            }
            if !ins.stale_docs.is_empty() {
                println!("possibly stale docs (mentioned files changed since):");
                for d in ins.stale_docs.iter().take(SHOW) {
                    println!(
                        "  [{}] {} ({}): {}",
                        d.severity.as_str(),
                        d.slug,
                        d.kind,
                        d.changed.join(", ")
                    );
                }
                showing(ins.stale_docs.len(), "stale docs");
            }
            if !ins.notes.is_empty() {
                println!("recent notes:");
                for (at, entity, text) in ins.notes.iter().take(SHOW) {
                    let age = now.saturating_sub(*at) / 1000;
                    println!("  [{age}s ago] {entity}: {text}");
                }
                showing(ins.notes.len(), "notes");
            }
            {
                let index = build_index(&store)?;
                let findings = brain_observe::coherence::check(&store, &index, prefix)
                    .map_err(|e| e.to_string())?;
                if !findings.is_empty() {
                    println!("coherence findings:");
                    for f in findings.iter().take(SHOW) {
                        println!("  {f}");
                    }
                    showing(findings.len(), "findings");
                }
            }
            if ins.series.len() > 1 {
                println!("growth (files/symbols/relations over refreshes):");
                for (at, f, s, r) in &ins.series {
                    let age = now.saturating_sub(*at) / 1000;
                    println!("  -{age:>6}s  {f} files  {s} symbols  {r} relations");
                }
            }
            Ok(())
        }
        Some("tests") => {
            let prefix = args
                .get(1)
                .filter(|a| !a.starts_with("--"))
                .ok_or("usage: brain twin tests <prefix> [--json]")?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let mut files: Vec<(String, String, String, String, Vec<String>)> = Vec::new();
            for (name, node) in store.namespace().map_err(|e| e.to_string())? {
                let Some(rel) = name.strip_prefix(&format!("{prefix}/")) else {
                    continue;
                };
                let Ok(Object::Entity { id: sid, .. }) = store.get(&node) else {
                    continue;
                };
                let Some(framework) =
                    brain_observe::twin::latest(&index, &store, &sid, "test_framework")
                        .map_err(|e| e.to_string())?
                else {
                    continue;
                };
                let declared = brain_observe::twin::latest(&index, &store, &sid, "tests_declared")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_else(|| "0".to_string());
                let role = brain_observe::twin::latest(&index, &store, &sid, "file_role")
                    .map_err(|e| e.to_string())?
                    .map(|_| "test file")
                    .unwrap_or("inline tests");
                let covers: Vec<String> = relation_targets(&store, &index, &sid, "covers")?
                    .iter()
                    .map(|t| entity_label(&store, &index, t))
                    .collect();
                files.push((rel.to_string(), framework, declared, role.to_string(), covers));
            }
            let failing = brain_observe::testing::failing_cases(&store, &index, prefix)
                .map_err(|e| e.to_string())?;
            if wants_json(args) {
                let rows: Vec<serde_json::Value> = files
                    .iter()
                    .map(|(file, framework, declared, role, covers)| {
                        serde_json::json!({
                            "file": file,
                            "framework": framework,
                            "declared": declared,
                            "role": role,
                            "covers": covers,
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::json!({"files": rows, "failing": failing})
                );
                return Ok(());
            }
            for (rel, framework, declared, role, covers) in files {
                let covering = if covers.is_empty() {
                    String::new()
                } else {
                    format!("  covers {}", covers.join(", "))
                };
                println!("{rel}  [{framework}] {declared} test(s), {role}{covering}");
            }
            if !failing.is_empty() {
                println!("failing now:");
                for name in failing {
                    println!("  ✗ {name}");
                }
            }
            Ok(())
        }
        Some("stale") => {
            let prefix = args
                .get(1)
                .filter(|a| !a.starts_with("--"))
                .ok_or("usage: brain twin stale <prefix> [--json]")?;
            let store = open_store()?;
            let ins = brain_observe::twin::insights(&store, prefix).map_err(|e| e.to_string())?;
            if wants_json(args) {
                println!(
                    "{}",
                    serde_json::to_string(&ins.stale_docs).map_err(|e| e.to_string())?
                );
                return Ok(());
            }
            if ins.stale_docs.is_empty() {
                println!("no stale docs under {prefix}");
            }
            for d in &ins.stale_docs {
                println!(
                    "[{}] {} ({}) — changed since doc updated or acknowledged:",
                    d.severity.as_str(),
                    d.slug,
                    d.kind
                );
                for f in &d.changed {
                    println!("  {f}");
                }
            }
            let warns = ins
                .stale_docs
                .iter()
                .filter(|d| d.severity == brain_observe::twin::Severity::Warn)
                .count();
            if !ins.stale_docs.is_empty() {
                println!(
                    "({warns} warn, {} info; reviewed-and-still-accurate? `brain adr|plan|artifact ack`)",
                    ins.stale_docs.len() - warns
                );
            }
            Ok(())
        }
        Some("config") => {
            let usage = "usage: brain twin config <prefix> [--add-extensions csv]";
            let prefix = args.get(1).filter(|a| !a.starts_with("--")).ok_or(usage)?;
            let add: Vec<String> = args
                .iter()
                .position(|a| a == "--add-extensions")
                .and_then(|i| args.get(i + 1))
                .map(|v| vec![v.clone()])
                .unwrap_or_default();
            let store = open_store()?;
            if add.is_empty() {
                let index = build_index(&store)?;
                let repo = brain_core::ids::StableId::derive(&["repo", prefix.as_str()]);
                let exts = brain_observe::twin::latest(&index, &store, &repo, "ingest_extensions")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_else(|| "(none beyond built-ins)".to_string());
                println!("extra ingest extensions for {prefix}: {exts}");
                return Ok(());
            }
            match brain_observe::twin::add_ingest_extensions(&store, prefix, &add)
                .map_err(|e| e.to_string())?
            {
                Some(csv) => println!(
                    "extra ingest extensions for {prefix}: {csv} (next refresh ingests them)"
                ),
                None => println!("no change — already taught"),
            }
            Ok(())
        }
        Some("search") => {
            let needle = args.get(1).ok_or("usage: brain twin search <substring>")?;
            let store = open_store()?;
            for (name, id) in store.namespace().map_err(|e| e.to_string())? {
                if name.contains(needle.as_str()) {
                    println!("{name}  ->  {id:?}");
                }
            }
            Ok(())
        }
        _ => Err(usage.to_string()),
    }
}

pub(crate) fn cmd_note(args: &[String]) -> Result<(), String> {
    let usage = "usage: brain note <name> <text...> [--kind learning|dead-end|gap|decision-pending]";
    let kind = args
        .iter()
        .position(|a| a == "--kind")
        .map(|i| args.get(i + 1).cloned().ok_or("--kind needs a value"))
        .transpose()?;
    if let Some(k) = kind.as_deref() {
        if !brain_observe::twin::NOTE_KINDS.contains(&k) {
            return Err(format!(
                "unknown note kind '{k}' ({})\n{usage}",
                brain_observe::twin::NOTE_KINDS.join("|")
            ));
        }
    }
    let pos = positional(args);
    let (name, rest) = match pos.as_slice() {
        [name, rest @ ..] if !rest.is_empty() => (*name, rest),
        _ => return Err(usage.to_string()),
    };
    let text = rest
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let store = open_store()?;
    let sid = entity_sid(&store, name)?;
    match kind {
        Some(k) => {
            brain_observe::twin::add_note_kinded(&store, &sid, &k, &text)
                .map_err(|e| e.to_string())?;
            println!("noted on {name} [{k}]");
        }
        None => {
            brain_observe::twin::add_note(&store, &sid, &text).map_err(|e| e.to_string())?;
            println!("noted on {name}");
        }
    }
    Ok(())
}

pub(crate) fn cmd_notes(args: &[String]) -> Result<(), String> {
    let name = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("usage: brain notes <name> [--top N] [--json]")?;
    let top = parse_top(args, usize::MAX)?;
    let store = open_store()?;
    let index = build_index(&store)?;
    let sid = entity_sid(&store, name)?;
    let mut notes = brain_observe::twin::notes(&index, &store, &sid).map_err(|e| e.to_string())?;
    // `--top N` keeps the newest N, still in true (oldest-first) order.
    if notes.len() > top {
        notes.drain(..notes.len() - top);
    }
    if wants_json(args) {
        let rows: Vec<serde_json::Value> = notes
            .iter()
            .map(|(at, text)| match brain_observe::twin::note_kind(text) {
                Some((kind, body)) => {
                    serde_json::json!({"at_ms": at, "kind": kind, "text": body})
                }
                None => serde_json::json!({"at_ms": at, "text": text}),
            })
            .collect();
        println!("{}", serde_json::Value::Array(rows));
        return Ok(());
    }
    if notes.is_empty() {
        println!("no notes on {name}");
    }
    for (at, text) in notes {
        println!("{at}  {text}");
    }
    Ok(())
}

/// `brain baseline ...` — name a moment so it can be asked about later.
pub(crate) fn cmd_baseline(args: &[String]) -> Result<(), String> {
    let usage = "usage: brain baseline add <prefix> <name> [--at <ms|30m|2h|1d|commit>] | list <prefix>";
    match args.first().map(String::as_str) {
        Some("add") => {
            let (prefix, name) = match (args.get(1), args.get(2)) {
                (Some(p), Some(n)) if !n.starts_with("--") => (p, n),
                _ => return Err(usage.to_string()),
            };
            let store = open_store()?;
            let index = build_index(&store)?;
            let at = args
                .iter()
                .position(|a| a == "--at")
                .and_then(|i| args.get(i + 1));
            let at_ms = match at {
                Some(when) => resolve_when(&store, &index, prefix, when)?,
                None => now_ms(),
            };
            brain_observe::baseline::add(&store, &index, prefix, name, at_ms)?;
            println!(
                "baseline '{name}' names that moment — `brain twin at {prefix} {name}` reads it back"
            );
            Ok(())
        }
        Some("list") => {
            let prefix = args.get(1).ok_or(usage)?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let baselines = brain_observe::baseline::list(&store, &index, prefix)
                .map_err(|e| e.to_string())?;
            if baselines.is_empty() {
                println!("no baselines yet — brain baseline add {prefix} <name> records one");
            }
            for b in baselines {
                println!("{}  {}", b.at_ms, b.name);
            }
            Ok(())
        }
        _ => Err(usage.to_string()),
    }
}

pub(crate) fn cmd_observations(args: &[String]) -> Result<(), String> {
    let arg = args.first().ok_or("usage: brain observations <name>")?;
    let store = open_store()?;
    let node = resolve_arg(&store, arg)?;
    let stable = match store.get(&node).map_err(|e| e.to_string())? {
        Object::Entity { id, .. } => id,
        other => {
            return Err(format!(
                "'{arg}' is not an entity (found {})",
                kind_of(&other)
            ))
        }
    };
    let index = build_index(&store)?;
    let mut rows = Vec::new();
    for obs_id in index.observations_of(&stable) {
        if let Object::Observation {
            property,
            value,
            source,
            observed_at_ms,
            ..
        } = store.get(&obs_id).map_err(|e| e.to_string())?
        {
            rows.push((observed_at_ms, property, value, source));
        }
    }
    rows.sort();
    for (at, property, value, source) in rows {
        println!("{at}  {property} = {value}  [{source}]");
    }
    Ok(())
}
