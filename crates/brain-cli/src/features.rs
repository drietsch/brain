//! Features: what the system claims to do, and whether the records agree.

use crate::support::*;

/// One line of a feature tree: readiness, then the score in its own terms.
pub(crate) fn print_part(title: &str, slug: &str, done: bool, score: (usize, usize), lead: &str) {
    let (met, total) = score;
    let mark = if done { "✓" } else { "·" };
    let tally = if total == 0 {
        "nothing to check".to_string()
    } else {
        format!("{met}/{total}")
    };
    println!("{lead}{mark} {title}  ({slug})  {tally}");
}

/// The parts under a feature, drawn with box characters so depth reads.
pub(crate) fn print_parts(parts: &[brain_observe::features::PartReport], lead: &str) {
    for (index, part) in parts.iter().enumerate() {
        let last = index + 1 == parts.len();
        let branch = if last { "└ " } else { "├ " };
        print_part(
            &part.title,
            &part.slug,
            part.done,
            (part.met, part.total),
            &format!("{lead}{branch}"),
        );
        let deeper = format!("{lead}{}", if last { "  " } else { "│ " });
        print_parts(&part.parts, &deeper);
    }
}

/// `brain feature ...` — the registry: features as entities, links as edges.
pub(crate) fn cmd_feature(args: &[String]) -> Result<(), String> {
    use brain_observe::features;
    let usage = "usage: brain feature add <prefix> <slug> [--title T] [--status S] [--part-of <parent>] | \
                 feature link <prefix> <slug> <predicate> <target> [--kind k] | \
                 feature list <prefix> | feature matrix <prefix> | feature tree <prefix> [slug]";
    match args.first().map(String::as_str) {
        Some("add") => {
            let (prefix, slug) = match (args.get(1), args.get(2)) {
                (Some(p), Some(s)) => (p, s),
                _ => return Err(usage.to_string()),
            };
            let mut title = slug.clone();
            let mut status = "planned".to_string();
            let mut part_of: Option<String> = None;
            let mut it = args[3..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--title" => title = it.next().cloned().unwrap_or(title),
                    "--status" => status = it.next().cloned().unwrap_or(status),
                    "--part-of" => part_of = it.next().cloned(),
                    other => return Err(format!("unexpected argument '{other}'\n{usage}")),
                }
            }
            let store = open_store()?;
            let (_, wrote) =
                features::add(&store, prefix, slug, &title, &status).map_err(|e| e.to_string())?;
            let state = if wrote { "recorded" } else { "unchanged" };
            println!("feature '{slug}' {state} under {prefix} (status: {status})");

            // Creating a part and attaching it is one act.
            if let Some(parent) = part_of {
                let index = build_index(&store)?;
                let (parent_sid, _) =
                    features::resolve_target_as(&store, &index, prefix, &parent, Some("feature"))
                        .map_err(|e| e.to_string())?
                        .ok_or_else(|| {
                            format!("no feature '{parent}' under {prefix} — register it first")
                        })?;
                let linked = features::link(&store, prefix, slug, features::PART_OF, &parent_sid)
                    .map_err(|e| e.to_string())?;
                println!(
                    "  {} part of '{parent}'",
                    if linked { "now" } else { "already" }
                );
            }
            Ok(())
        }
        Some("tree") => {
            let prefix = args.get(1).ok_or(usage)?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let only = args.get(2).filter(|a| !a.starts_with("--"));

            // Roots are features nothing else claims as a parent, unless
            // one was named.
            let mut roots: Vec<String> = Vec::new();
            for row in features::list(&store, &index, prefix).map_err(|e| e.to_string())? {
                if let Some(want) = only {
                    if row.slug == **want {
                        roots.push(row.slug);
                    }
                    continue;
                }
                let sid = features::feature_sid(prefix, &row.slug);
                if features::parent(&store, &index, &sid)
                    .map_err(|e| e.to_string())?
                    .is_none()
                {
                    roots.push(row.slug);
                }
            }
            if roots.is_empty() {
                println!("no features under {prefix}");
                return Ok(());
            }
            if wants_json(args) {
                let mut out = Vec::new();
                for slug in &roots {
                    let report = features::evaluate(&store, &index, prefix, slug)
                        .map_err(|e| e.to_string())?;
                    let title = features::list(&store, &index, prefix)
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .find(|r| r.slug == *slug)
                        .map(|r| r.title)
                        .unwrap_or_else(|| slug.clone());
                    let (met, total) = report.score();
                    let mut v = serde_json::to_value(&report).map_err(|e| e.to_string())?;
                    v["slug"] = serde_json::Value::String(slug.clone());
                    v["title"] = serde_json::Value::String(title);
                    v["met"] = serde_json::Value::from(met);
                    v["total"] = serde_json::Value::from(total);
                    out.push(v);
                }
                println!("{}", serde_json::Value::Array(out));
                return Ok(());
            }
            for slug in roots {
                let report =
                    features::evaluate(&store, &index, prefix, &slug).map_err(|e| e.to_string())?;
                let title = features::list(&store, &index, prefix)
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .find(|r| r.slug == slug)
                    .map(|r| r.title)
                    .unwrap_or_else(|| slug.clone());
                print_part(&title, &slug, report.done, report.score(), "");
                print_parts(&report.parts, "");
                if let Some(blocking) = &report.blocked_by {
                    println!("  waiting on: {blocking}");
                }
            }
            Ok(())
        }
        Some("link") => {
            let pos = positional(&args[1..]);
            let (prefix, slug, predicate, target) = match pos.as_slice() {
                [p, s, pr, t] => (*p, *s, *pr, *t),
                _ => return Err(usage.to_string()),
            };
            // A composition edge must land on a feature; otherwise a part
            // named like an ADR would silently attach to the ADR.
            let mut want = args
                .iter()
                .position(|a| a == "--kind")
                .and_then(|i| args.get(i + 1))
                .map(String::as_str);
            if want.is_none() && predicate == features::PART_OF {
                want = Some("feature");
            }
            let store = open_store()?;
            let index = build_index(&store)?;
            let (target_sid, kind) =
                features::resolve_target_as(&store, &index, prefix, target, want)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| match want {
                        Some(k) => format!("no {k} '{target}' under {prefix}"),
                        None => format!("no twinned entity matches '{target}' (file path, or the slug of any registered kind)"),
                    })?;
            // Advisory link vocabulary: warn (never refuse) when the
            // feature kind declares allowed predicates and this one is not
            // among them.
            let reg = brain_observe::kinds::registry(&store, &index).map_err(|e| e.to_string())?;
            if let Some(def) = reg.get("feature") {
                if !def.links.is_empty() && !def.links.contains(predicate) {
                    eprintln!(
                        "warning: '{predicate}' is not in the feature kind's link vocabulary [{}] — linked anyway",
                        def.links.join(", ")
                    );
                }
            }
            let wrote = features::link(&store, prefix, slug, predicate, &target_sid)
                .map_err(|e| e.to_string())?;
            let state = if wrote { "linked" } else { "already linked" };
            println!("{slug} -{predicate}-> {target} ({kind}): {state}");
            Ok(())
        }
        Some("list") => {
            let prefix = args.get(1).ok_or(usage)?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let rows = features::list(&store, &index, prefix).map_err(|e| e.to_string())?;
            if rows.is_empty() {
                println!("no features under {prefix}");
            }
            for row in rows {
                let report = features::evaluate(&store, &index, prefix, &row.slug)
                    .map_err(|e| e.to_string())?;
                let (met, total) = report.score();
                let done = if report.done { " ✓ done" } else { "" };
                let sid = features::feature_sid(prefix, &row.slug);
                let under = features::parent(&store, &index, &sid)
                    .map_err(|e| e.to_string())?
                    .map(|(_, parent)| format!("  part of {parent}"))
                    .unwrap_or_default();
                let terms = if report.by_parts() { "parts" } else { "linked" };
                println!(
                    "[{}] {}: {}  ({met}/{total} {terms}{done}){under}",
                    row.status, row.slug, row.title,
                );
            }
            Ok(())
        }
        Some("matrix") => {
            let prefix = args.get(1).ok_or(usage)?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let dod = features::dod(&store, &index).map_err(|e| e.to_string())?;
            let rows = features::list(&store, &index, prefix).map_err(|e| e.to_string())?;
            if rows.is_empty() {
                println!("no features under {prefix}");
                return Ok(());
            }
            let width = rows.iter().map(|r| r.slug.len()).max().unwrap_or(8).max(8);
            let header: Vec<String> = dod.iter().map(|d| d.replace("_by", "")).collect();
            println!("{:<width$}  {}  done", "feature", header.join("  "));
            for row in rows {
                let report = features::evaluate(&store, &index, prefix, &row.slug)
                    .map_err(|e| e.to_string())?;
                let cells: Vec<String> = report
                    .checks
                    .iter()
                    .zip(&header)
                    .map(|(c, h)| {
                        let mark = if c.count > 0 { "✓" } else { "✗" };
                        format!("{mark:^w$}", w = h.len().max(1))
                    })
                    .collect();
                let done = if report.done { "✓" } else { "✗" };
                println!("{:<width$}  {}  {done}", row.slug, cells.join("  "));
            }
            Ok(())
        }
        _ => Err(usage.to_string()),
    }
}

/// `brain done <prefix> <slug>` — evaluate a feature against the DoD and
/// record the outcome as a guarded observation.
pub(crate) fn cmd_done(args: &[String]) -> Result<(), String> {
    use brain_observe::features;
    let (prefix, slug) = match (args.first(), args.get(1)) {
        (Some(p), Some(s)) => (p, s),
        _ => return Err("usage: brain done <prefix> <feature-slug>".to_string()),
    };
    let store = open_store()?;
    let index = build_index(&store)?;
    let report = features::evaluate(&store, &index, prefix, slug).map_err(|e| e.to_string())?;

    if report.by_parts() {
        // A feature with parts is judged by its parts; its own links are
        // still shown, but they are evidence, not the verdict.
        println!("judged by its {} part(s):", report.parts.len());
        print_parts(&report.parts, "");
        let linked = report.checks.iter().filter(|c| c.count > 0).count();
        if linked > 0 {
            println!(
                "(also linked directly: {})",
                report
                    .checks
                    .iter()
                    .filter(|c| c.count > 0)
                    .map(|c| c.predicate.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    } else {
        for check in &report.checks {
            let mark = if check.count > 0 { "✓" } else { "✗" };
            println!("{mark} {}  ({} link(s))", check.predicate, check.count);
        }
    }

    println!(
        "{}: {}",
        slug,
        if report.done { "DONE" } else { "not done" }
    );
    if let Some(blocking) = &report.blocked_by {
        println!("waiting on: {blocking}");
    }
    features::record_done(&store, &index, prefix, slug, &report).map_err(|e| e.to_string())?;
    Ok(())
}
