//! Brain functions: wake, attend, sleep and the rest of the orientation verbs.

use crate::support::*;

/// `brain tidy` — the brain cleans up: advisory scan, safe fixes via
/// governed changes, deletion only by explicit `--rm`.
pub(crate) fn cmd_tidy(args: &[String]) -> Result<(), String> {
    let dir = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| ".".to_string());
    let prefix = parse_prefix(args);
    let apply_fix = args.iter().any(|a| a == "--fix");
    let caps: Vec<String> = args
        .iter()
        .enumerate()
        .filter(|(_, a)| *a == "--cap")
        .filter_map(|(i, _)| args.get(i + 1).cloned())
        .collect();
    let rm = args
        .iter()
        .position(|a| a == "--rm")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let store = open_store()?;
    let index = build_index(&store)?;
    let root = std::path::Path::new(&dir);

    if let Some(path) = rm {
        let ok = brain_observe::tidy::remove_path(&store, root, &path, &caps)
            .map_err(|e| e.to_string())?;
        if ok {
            println!("removed {path} (intent + receipt recorded)");
        } else {
            return Err(format!("removal of {path} failed — see the receipt"));
        }
        return Ok(());
    }

    let findings =
        brain_observe::tidy::scan(&store, &index, root, &prefix).map_err(|e| e.to_string())?;
    if findings.is_empty() {
        println!("nothing to tidy under {prefix}");
        return Ok(());
    }
    for f in &findings {
        println!(
            "[{}] {} — {}\n    fix: {}",
            f.category, f.path, f.detail, f.fix
        );
    }
    if !apply_fix {
        let fixable = findings.iter().filter(|f| f.fixable).count();
        println!(
            "({} finding(s), {fixable} fixable — `brain tidy {dir} --prefix {prefix} --fix --cap fs`; deletion only via --rm <path>)",
            findings.len()
        );
        return Ok(());
    }
    let (fixed, skipped) = brain_observe::tidy::fix(&store, root, &prefix, &findings, &caps)
        .map_err(|e| e.to_string())?;
    for m in &fixed {
        println!("fixed: {m}");
    }
    for (path, why) in &skipped {
        println!("skipped: {path} — {why}");
    }
    println!("({} fixed, {} skipped)", fixed.len(), skipped.len());
    Ok(())
}

/// `brain eyes` — the read-only human projection served from this binary.
pub(crate) fn cmd_eyes(args: &[String]) -> Result<(), String> {
    let mut config = brain_eyes::Config {
        store_root: std::env::var("BRAIN_STORE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from(".brain")),
        ..brain_eyes::Config::default()
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--prefix" => {
                config.prefix = it.next().cloned().ok_or("--prefix needs a value")?;
            }
            "--bind" => {
                config.bind = it.next().cloned().ok_or("--bind needs an address")?;
            }
            "--port" => {
                config.port = it
                    .next()
                    .and_then(|value| value.parse().ok())
                    .ok_or("--port needs a number from 0 to 65535")?;
            }
            // Where file-backed bodies are read from. Defaults to the
            // working directory, which is the repo the twin observes.
            "--root" => {
                config.content_root = it
                    .next()
                    .map(std::path::PathBuf::from)
                    .ok_or("--root needs a directory")?;
            }
            other => {
                return Err(format!(
                    "unexpected argument '{other}'\nusage: brain eyes [--prefix P] [--bind IP] [--port N] [--root DIR]"
                ));
            }
        }
    }
    brain_eyes::serve(config)
}

/// `brain attend` — the attention organ: what deserves attention now.
/// `brain wake <prefix>` — one command, the whole present.
pub(crate) fn cmd_wake(args: &[String]) -> Result<(), String> {
    let prefix = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("usage: brain wake <prefix> [--full] [--json]")?;
    let full = args.iter().any(|a| a == "--full");
    let store = open_store()?;
    let index = build_index(&store)?;
    if wants_json(args) {
        let o = brain_observe::wake::orientation(&store, &index, prefix)
            .map_err(|e| e.to_string())?;
        println!("{}", serde_json::to_string(&o).map_err(|e| e.to_string())?);
        return Ok(());
    }
    let text =
        brain_observe::wake::wake(&store, &index, prefix, full).map_err(|e| e.to_string())?;
    println!("{text}");
    Ok(())
}

pub(crate) fn cmd_attend(args: &[String]) -> Result<(), String> {
    let prefix = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("usage: brain attend <prefix> [--top N] [--json]")?;
    let top = parse_top(args, 10)?;
    let store = open_store()?;
    let index = build_index(&store)?;
    let ranked =
        brain_observe::attention::attend(&store, &index, prefix).map_err(|e| e.to_string())?;
    if wants_json(args) {
        let rows: Vec<_> = ranked.iter().take(top).collect();
        println!("{}", serde_json::to_string(&rows).map_err(|e| e.to_string())?);
        return Ok(());
    }
    if ranked.is_empty() {
        println!("nothing demands attention under {prefix}");
    }
    for (i, a) in ranked.iter().take(top).enumerate() {
        println!(
            "{:>2}. [{:>3}] {} ({})  — {}",
            i + 1,
            a.score,
            a.label,
            a.kind,
            a.reasons.join(", ")
        );
    }
    Ok(())
}

/// `brain spine` — what each feature reaches, and what nothing claims.
pub(crate) fn cmd_spine(args: &[String]) -> Result<(), String> {
    let prefix = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("usage: brain spine <prefix> [--unclaimed <kind>] [--json]")?;
    let want = args
        .iter()
        .position(|a| a == "--unclaimed")
        .and_then(|i| args.get(i + 1));
    let store = open_store()?;
    let index = build_index(&store)?;
    let spine = brain_observe::spine::build(&store, &index, prefix).map_err(|e| e.to_string())?;

    if wants_json(args) {
        if let Some(kind) = want {
            let rows: Vec<String> = spine
                .unclaimed(kind)
                .iter()
                .map(|sid| brain_observe::twin::sid_label(&index, &store, sid))
                .collect();
            println!("{}", serde_json::to_string(&rows).map_err(|e| e.to_string())?);
            return Ok(());
        }
        let features: Vec<serde_json::Value> = spine
            .slugs()
            .map(|slug| {
                let reach = spine.reach(slug).expect("just listed");
                let by_kind: serde_json::Map<String, serde_json::Value> = reach
                    .by_kind
                    .iter()
                    .map(|(kind, rows)| (kind.clone(), serde_json::Value::from(rows.len())))
                    .collect();
                serde_json::json!({
                    "slug": slug,
                    "files": reach.files.len(),
                    "reaches": by_kind,
                })
            })
            .collect();
        let (claimed, total) = spine.claimed_total();
        let census: Vec<serde_json::Value> = spine
            .census()
            .iter()
            .map(|r| serde_json::json!({"kind": r.kind, "claimed": r.claimed, "total": r.total}))
            .collect();
        let uncorroborated: Vec<serde_json::Value> = spine
            .uncorroborated()
            .iter()
            .map(|r| {
                serde_json::json!({
                    "slug": r.slug,
                    "predicate": r.predicate,
                    "targets": r.targets.len(),
                    "why": r.why,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "features": features,
                "coverage": {"claimed": claimed, "total": total, "census": census},
                "uncorroborated": uncorroborated,
            })
        );
        return Ok(());
    }

    if !spine.asked() {
        println!("no feature under {prefix} declares anything yet");
        return Ok(());
    }

    if let Some(kind) = want {
        let rows = spine.unclaimed(kind);
        if rows.is_empty() {
            println!("every {kind} under {prefix} is claimed by a feature");
        }
        for sid in rows {
            println!("  {}", brain_observe::twin::sid_label(&index, &store, sid));
        }
        return Ok(());
    }

    for slug in spine.slugs().map(str::to_string).collect::<Vec<_>>() {
        let reach = spine.reach(&slug).expect("just listed");
        let counts: Vec<String> = reach
            .by_kind
            .iter()
            .map(|(kind, rows)| format!("{} {kind}", rows.len()))
            .collect();
        println!(
            "{slug}  {} file(s) declared -> {}",
            reach.files.len(),
            if counts.is_empty() {
                "nothing".to_string()
            } else {
                counts.join(", ")
            }
        );
    }
    let (claimed, total) = spine.claimed_total();
    println!("\ncoverage: {claimed} of {total} records are claimed by a feature");
    for row in spine.census() {
        println!("  {:<16} {}/{}", row.kind, row.claimed, row.total);
    }
    if !spine.uncorroborated().is_empty() {
        println!("\nlinked, but nothing observed corroborates it:");
        for row in spine.uncorroborated() {
            println!(
                "  {} [{}] ×{} — {}",
                row.slug,
                row.predicate,
                row.targets.len(),
                row.why
            );
        }
    }
    Ok(())
}

/// `brain sleep` — the consolidation organ: distill history into memory.
pub(crate) fn cmd_sleep(args: &[String]) -> Result<(), String> {
    let prefix = args.first().ok_or("usage: brain sleep <prefix>")?;
    let store = open_store()?;
    let report = brain_observe::sleep::sleep(&store, prefix).map_err(|e| e.to_string())?;
    if report.wrote {
        println!("slept: {}", report.summary);
    } else {
        println!("{}", report.summary);
    }
    Ok(())
}

/// `brain related` — the association organ: soft, derived, disposable.
pub(crate) fn cmd_related(args: &[String]) -> Result<(), String> {
    let name = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("usage: brain related <name> [--top N] [--json]")?;
    let top = parse_top(args, 10)?;
    let store = open_store()?;
    let index = build_index(&store)?;
    let sid = entity_sid(&store, name)?;
    let prefix = twin_prefix_of(&store, name)?;
    let assoc = brain_observe::assoc::AssocIndex::build(&store, &index, &prefix)
        .map_err(|e| e.to_string())?;
    let related = assoc.related(&sid);
    if wants_json(args) {
        let rows: Vec<serde_json::Value> = related
            .into_iter()
            .take(top)
            .map(|(label, score, reasons)| {
                serde_json::json!({"label": label, "score": score, "reasons": reasons})
            })
            .collect();
        println!("{}", serde_json::Value::Array(rows));
        return Ok(());
    }
    if related.is_empty() {
        println!("no associations for {name} (yet — associations grow with history)");
    }
    for (label, score, reasons) in related.into_iter().take(top) {
        println!("[{score:>3}] {label}  — {}", reasons.join(", "));
    }
    Ok(())
}

/// `brain before` — the pre-edit briefing: what depends on this, what
/// covers it, what constrains it, what past sessions learned here, and
/// whether it may be written at all. One command instead of five.
pub(crate) fn cmd_before(args: &[String]) -> Result<(), String> {
    let name = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("usage: brain before <name> [--json]")?;
    let store = open_store()?;
    let index = build_index(&store)?;
    let sid = entity_sid(&store, name)?;
    let prefix = twin_prefix_of(&store, name)?;
    let briefing = brain_observe::briefing::before(&store, &index, &prefix, name, &sid)
        .map_err(|e| e.to_string())?;
    if wants_json(args) {
        println!(
            "{}",
            serde_json::to_string(&briefing).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    println!("{}", brain_observe::briefing::render(&briefing));
    Ok(())
}

/// `brain next` — the future leg: the ranked work queue, derived from
/// everything the graph knows is failing, unsettled, rotting, or open.
pub(crate) fn cmd_next(args: &[String]) -> Result<(), String> {
    let prefix = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("usage: brain next <prefix> [--top N] [--json]")?;
    let top = parse_top(args, 10)?;
    let store = open_store()?;
    let index = build_index(&store)?;
    let items =
        brain_observe::agenda::queue(&store, &index, prefix).map_err(|e| e.to_string())?;
    if wants_json(args) {
        let rows: Vec<_> = items.iter().take(top).collect();
        println!("{}", serde_json::to_string(&rows).map_err(|e| e.to_string())?);
        return Ok(());
    }
    println!("{}", brain_observe::agenda::render(&items, prefix, top));
    Ok(())
}

/// `brain find` — where is the thing that does X: lexical match over
/// paths, symbol names, doc titles, and notes, ranked by graph centrality.
pub(crate) fn cmd_find(args: &[String]) -> Result<(), String> {
    let usage = "usage: brain find <prefix> <query...> [--top N] [--json]";
    let pos = positional(args);
    let (prefix, terms) = match pos.as_slice() {
        [prefix, terms @ ..] if !terms.is_empty() => (*prefix, terms),
        _ => return Err(usage.to_string()),
    };
    let query = terms
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let top = parse_top(args, 10)?;
    let store = open_store()?;
    let index = build_index(&store)?;
    let hits = brain_observe::find::find(&store, &index, prefix, &query)
        .map_err(|e| e.to_string())?;
    if wants_json(args) {
        let rows: Vec<_> = hits.iter().take(top).collect();
        println!("{}", serde_json::to_string(&rows).map_err(|e| e.to_string())?);
        return Ok(());
    }
    println!("{}", brain_observe::find::render(&hits, &query, prefix, top));
    Ok(())
}

/// `brain can-i` — the authoring gate as a question. Exit 0 = write the
/// file; exit 3 = the graph owns it, and the answer names the command
/// that edits it. Works for paths the twin has never seen.
pub(crate) fn cmd_can_i(args: &[String]) -> Result<(), String> {
    let name = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("usage: brain can-i <name-or-path> [--prefix <p>] [--json]")?;
    let store = open_store()?;
    let index = build_index(&store)?;
    // A bound name carries its own prefix; a bare repo path gets one from
    // --prefix (default twin) and is judged by capture rules alone.
    let (prefix, sid, rel) = match entity_sid(&store, name) {
        Ok(sid) => {
            let prefix = twin_prefix_of(&store, name)?;
            let rel = name
                .strip_prefix(&format!("{prefix}/"))
                .unwrap_or(name)
                .to_string();
            (prefix, sid, rel)
        }
        Err(_) => {
            let prefix = parse_prefix(&args[1..]);
            let sid = brain_core::ids::StableId::derive(&["file", name]);
            (prefix, sid, name.clone())
        }
    };
    let access = brain_observe::briefing::write_access(&store, &index, &prefix, &sid, &rel)
        .map_err(|e| e.to_string())?;
    if wants_json(args) {
        let mut v = serde_json::to_value(&access).map_err(|e| e.to_string())?;
        v["name"] = serde_json::Value::String(name.clone());
        println!("{v}");
    }
    use brain_observe::briefing::WriteAccess;
    match access {
        WriteAccess::File => {
            if !wants_json(args) {
                println!("yes — a plain file; the twin observes changes on refresh");
            }
            Ok(())
        }
        WriteAccess::Captured { kind } => {
            if !wants_json(args) {
                println!("yes — captured as {kind} on refresh (file-first)");
            }
            Ok(())
        }
        WriteAccess::Projection {
            kind,
            slug,
            edit_via,
        } => Err(format!(
            "refused: {name} is a read-only projection of {kind}/{slug} — edit via `{edit_via}`"
        )),
    }
}
