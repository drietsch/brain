//! Deliverables — decisions, plans, templates, artifacts, assets — authored through the graph.

use brain_core::object::Object;
use brain_index::{Index, MemIndex};
use brain_store::{now_ms, Store};
use std::collections::BTreeSet;
use crate::support::*;

/// `brain adr ...` / `brain plan ...` — decisions and plans in the twin.
pub(crate) fn cmd_doc(args: &[String], kind: brain_observe::docs::DocKind) -> Result<(), String> {
    use brain_observe::{docs, twin};
    let noun = kind.as_str();
    let cmd = match kind {
        docs::DocKind::Decision => "adr",
        docs::DocKind::Plan => "plan",
    };
    let usage = format!(
        "usage: brain {cmd} add <md-file> --prefix <p> [--title T]{} | \
         brain {cmd} list <prefix> [--all] | brain {cmd} show <prefix> <slug>{}",
        if cmd == "adr" { " [--status S]" } else { "" },
        if cmd == "plan" {
            " | brain plan done|abandon|reopen <prefix> <slug> [--why R]"
        } else {
            ""
        }
    );
    match args.first().map(String::as_str) {
        Some(op @ ("done" | "abandon" | "reopen")) if cmd == "plan" => {
            let (prefix, slug) = match (args.get(1), args.get(2)) {
                (Some(p), Some(s)) if !p.starts_with("--") && !s.starts_with("--") => (p, s),
                _ => return Err(usage),
            };
            let why = args
                .iter()
                .position(|a| a == "--why")
                .and_then(|i| args.get(i + 1))
                .map(String::as_str);
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = brain_core::ids::StableId::derive(&[noun, prefix, slug]);
            if index.entity_nodes(&sid).is_empty() {
                return Err(format!("no {noun} '{slug}' under {prefix}"));
            }
            let state = match op {
                "done" => brain_observe::lifecycle::Lifecycle::Done,
                "abandon" => brain_observe::lifecycle::Lifecycle::Abandoned,
                _ => brain_observe::lifecycle::Lifecycle::Active,
            };
            let wrote = brain_observe::lifecycle::set(&store, &index, &sid, state, why)
                .map_err(|e| e.to_string())?;
            let verb = if wrote { "now" } else { "already" };
            println!("plan '{slug}' {verb} {}", state.as_str());
            Ok(())
        }
        Some("ack") => {
            let (prefix, slug) = match (args.get(1), args.get(2)) {
                (Some(p), Some(s)) if !p.starts_with("--") && !s.starts_with("--") => (p, s),
                _ => return Err(usage),
            };
            let note = args
                .iter()
                .position(|a| a == "--note")
                .and_then(|i| args.get(i + 1))
                .map(String::as_str)
                .unwrap_or("reviewed, still accurate");
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = brain_core::ids::StableId::derive(&[noun, prefix, slug]);
            if index.entity_nodes(&sid).is_empty() {
                return Err(format!("no {noun} '{slug}' under {prefix}"));
            }
            brain_observe::twin::ack(&store, &sid, note).map_err(|e| e.to_string())?;
            println!("{noun} '{slug}' acknowledged — staleness clock reset");
            Ok(())
        }
        Some("add") => {
            let mut file = None;
            let mut prefix = None;
            let mut title = None;
            let mut status = None;
            let mut it = args[1..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--prefix" => prefix = it.next().cloned(),
                    "--title" => title = it.next().cloned(),
                    "--status" if cmd == "adr" => status = it.next().cloned(),
                    other if file.is_none() && !other.starts_with("--") => {
                        file = Some(other.to_string())
                    }
                    other => return Err(format!("unexpected argument '{other}'\n{usage}")),
                }
            }
            let file = file.ok_or_else(|| usage.clone())?;
            let prefix = prefix.ok_or_else(|| format!("--prefix is required\n{usage}"))?;
            let content =
                std::fs::read_to_string(&file).map_err(|e| format!("cannot read '{file}': {e}"))?;
            let slug = docs::slug_of(&file);
            let meta =
                docs::parse_content(kind, &slug, &content, title.as_deref(), status.as_deref());
            let store = open_store()?;
            let out = twin::add_doc(&store, &prefix, &meta, &content, "claude-code")
                .map_err(|e| e.to_string())?;
            let state = if out.wrote {
                "recorded"
            } else {
                "already recorded (unchanged)"
            };
            let status_part = meta
                .status
                .as_deref()
                .map(|s| format!(", status: {s}"))
                .unwrap_or_default();
            println!(
                "{noun} '{slug}' {state} under {prefix} (title: {}{status_part}, {} mention(s))",
                meta.title,
                out.mentions.len()
            );
            for m in &out.mentions {
                println!("  mentions {prefix}/{m}");
            }
            Ok(())
        }
        Some("list") => {
            let prefix = args
                .get(1)
                .filter(|a| !a.starts_with("--"))
                .ok_or_else(|| usage.clone())?;
            let all = args.iter().any(|a| a == "--all");
            let store = open_store()?;
            let index = build_index(&store)?;
            let now = now_ms();
            let mut seen = BTreeSet::new();
            let mut any = false;
            let mut hidden = 0usize;
            for node in index.entities_by_kind(noun) {
                let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
                    continue;
                };
                if labels.get("prefix") != Some(prefix) || !seen.insert(id.clone()) {
                    continue;
                }
                let (lc, _) =
                    brain_observe::lifecycle::of(&index, &store, &id).map_err(|e| e.to_string())?;
                if !lc.is_active() && !all {
                    hidden += 1;
                    continue;
                }
                let slug = labels.get("slug").cloned().unwrap_or_default();
                let title = brain_observe::twin::latest(&index, &store, &id, "title")
                    .map_err(|e| e.to_string())?
                    .or_else(|| labels.get("title").cloned())
                    .unwrap_or_else(|| slug.clone());
                let status = brain_observe::twin::latest(&index, &store, &id, "status")
                    .map_err(|e| e.to_string())?
                    .map(|s| format!("[{s}] "))
                    .unwrap_or_default();
                let age = brain_observe::twin::latest_at(&index, &store, &id, "content")
                    .map_err(|e| e.to_string())?
                    .map(|(at, _)| format!("{}s ago", now.saturating_sub(at) / 1000))
                    .unwrap_or_else(|| "?".to_string());
                let mentions = relation_targets(&store, &index, &id, "mentions")?.len();
                let tag = if lc.is_active() {
                    String::new()
                } else {
                    format!("  [{}]", lc.as_str())
                };
                println!("{status}{slug}: {title}  ({age}, {mentions} mention(s)){tag}");
                any = true;
            }
            if !any {
                println!(
                    "no {}{noun}s under {prefix}",
                    if all { "" } else { "active " }
                );
            }
            if hidden > 0 {
                println!("({hidden} non-active hidden — --all shows history)");
            }
            Ok(())
        }
        Some("show") => {
            let (prefix, slug) = match (args.get(1), args.get(2)) {
                (Some(p), Some(s)) => (p, s),
                _ => return Err(usage),
            };
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = brain_core::ids::StableId::derive(&[noun, prefix, slug]);
            let content = brain_observe::twin::latest(&index, &store, &sid, "content")
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("no {noun} '{slug}' under {prefix}"))?;
            print!("{content}");
            if !content.ends_with('\n') {
                println!();
            }
            let (lc, why) =
                brain_observe::lifecycle::of(&index, &store, &sid).map_err(|e| e.to_string())?;
            if !lc.is_active() {
                let detail = if why.is_empty() {
                    String::new()
                } else {
                    format!(" ({why})")
                };
                println!("--- lifecycle: {}{detail} ---", lc.as_str());
            }
            // Status timeline (decisions), oldest first.
            let mut statuses = Vec::new();
            for id in index.observations_of(&sid) {
                if let Object::Observation {
                    property,
                    value,
                    observed_at_ms,
                    ..
                } = store.get(&id).map_err(|e| e.to_string())?
                {
                    if property == "status" {
                        statuses.push((observed_at_ms, value));
                    }
                }
            }
            statuses.sort();
            if !statuses.is_empty() {
                println!("--- status timeline ---");
                for (at, s) in statuses {
                    println!("{at}  {s}");
                }
            }
            let mentions = relation_targets(&store, &index, &sid, "mentions")?;
            if !mentions.is_empty() {
                println!("--- mentions ---");
                for m in mentions {
                    println!("{}", entity_label(&store, &index, &m));
                }
            }
            Ok(())
        }
        _ => Err(usage),
    }
}

/// `brain skill ...` / `brain agentcfg ...` — the agent-operating layer.
pub(crate) fn cmd_agent_doc(args: &[String], kind: brain_observe::agents::AgentDocKind) -> Result<(), String> {
    use brain_observe::{agents, twin};
    let noun = kind.as_str();
    let cmd = match kind {
        agents::AgentDocKind::Skill => "skill",
        agents::AgentDocKind::Config => "agentcfg",
    };
    let usage = format!(
        "usage: brain {cmd} add <file> --prefix <p> [--agent A] [--role R] | \
         brain {cmd} list <prefix> | brain {cmd} show <prefix> <slug>"
    );
    match args.first().map(String::as_str) {
        Some("add") => {
            let mut file = None;
            let mut prefix = None;
            let mut agent = None;
            let mut role = None;
            let mut it = args[1..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--prefix" => prefix = it.next().cloned(),
                    "--agent" => agent = it.next().cloned(),
                    "--role" => role = it.next().cloned(),
                    other if file.is_none() && !other.starts_with("--") => {
                        file = Some(other.to_string())
                    }
                    other => return Err(format!("unexpected argument '{other}'\n{usage}")),
                }
            }
            let file = file.ok_or_else(|| usage.clone())?;
            let prefix = prefix.ok_or_else(|| format!("--prefix is required\n{usage}"))?;
            let content =
                std::fs::read_to_string(&file).map_err(|e| format!("cannot read '{file}': {e}"))?;
            // Detect slug/agent/role from the path conventions when they
            // apply, then let explicit flags override.
            let normalized = file.replace('\\', "/");
            let (slug, det_agent, det_role) = match agents::path_agent_doc(&normalized) {
                Some((k, slug, a, r)) if k == kind => (slug, a, r),
                _ => {
                    let stem = normalized.rsplit('/').next().unwrap_or(&normalized);
                    let stem = stem.strip_suffix(".md").unwrap_or(stem).to_lowercase();
                    let default_role = if cmd == "skill" {
                        "skill"
                    } else {
                        "instructions"
                    };
                    (stem, "generic".to_string(), default_role.to_string())
                }
            };
            let doc = agents::parse_agent_content(
                kind,
                &slug,
                agent.as_deref().unwrap_or(&det_agent),
                role.as_deref().unwrap_or(&det_role),
                &content,
            );
            let store = open_store()?;
            let out = twin::add_agent_doc(&store, &prefix, &doc, &content, "claude-code")
                .map_err(|e| e.to_string())?;
            let state = if out.wrote {
                "recorded"
            } else {
                "already recorded (unchanged)"
            };
            println!(
                "{noun} '{slug}' {state} under {prefix} (agent: {}, role: {}, {} mention(s))",
                doc.agent,
                doc.role,
                out.mentions.len()
            );
            for m in &out.mentions {
                println!("  mentions {prefix}/{m}");
            }
            Ok(())
        }
        Some("list") => {
            let prefix = args
                .get(1)
                .filter(|a| !a.starts_with("--"))
                .ok_or_else(|| usage.clone())?;
            let all = args.iter().any(|a| a == "--all");
            let store = open_store()?;
            let index = build_index(&store)?;
            let mut seen = BTreeSet::new();
            let mut any = false;
            let mut hidden = 0usize;
            for node in index.entities_by_kind(noun) {
                let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
                    continue;
                };
                if labels.get("prefix") != Some(prefix) || !seen.insert(id.clone()) {
                    continue;
                }
                let (lc, _) =
                    brain_observe::lifecycle::of(&index, &store, &id).map_err(|e| e.to_string())?;
                if !lc.is_active() && !all {
                    hidden += 1;
                    continue;
                }
                let slug = labels.get("slug").cloned().unwrap_or_default();
                let agent = brain_observe::twin::latest(&index, &store, &id, "agent")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_else(|| "generic".to_string());
                let role = brain_observe::twin::latest(&index, &store, &id, "role")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default();
                let desc = brain_observe::twin::latest(&index, &store, &id, "description")
                    .map_err(|e| e.to_string())?
                    .map(|d| format!("  — {d}"))
                    .unwrap_or_default();
                let tag = if lc.is_active() {
                    String::new()
                } else {
                    format!("  [{}]", lc.as_str())
                };
                println!("[{agent}] {slug} ({role}){desc}{tag}");
                any = true;
            }
            if !any {
                println!(
                    "no {}{noun}s under {prefix}",
                    if all { "" } else { "active " }
                );
            }
            if hidden > 0 {
                println!("({hidden} non-active hidden — --all shows history)");
            }
            Ok(())
        }
        Some("show") => {
            let (prefix, slug) = match (args.get(1), args.get(2)) {
                (Some(p), Some(s)) => (p, s),
                _ => return Err(usage),
            };
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = brain_core::ids::StableId::derive(&[noun, prefix, slug]);
            let content = brain_observe::twin::latest(&index, &store, &sid, "content")
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("no {noun} '{slug}' under {prefix}"))?;
            print!("{content}");
            if !content.ends_with('\n') {
                println!();
            }
            let mentions = relation_targets(&store, &index, &sid, "mentions")?;
            if !mentions.is_empty() {
                println!("--- mentions ---");
                for m in mentions {
                    println!("{}", entity_label(&store, &index, &m));
                }
            }
            Ok(())
        }
        _ => Err(usage),
    }
}

/// `brain template ...` — the deliverable contract, defined in the graph.
pub(crate) fn cmd_template(args: &[String]) -> Result<(), String> {
    use brain_observe::{templates, twin};
    let usage = "usage: brain template seed | list | show <slug> | \
                 set <slug> [--applies-to k] [--capture \"globs\"] [--fields \"spec\"] \
                 [--requires \"a,b\"] [--rot none|info|warn] [--placement P] [--enforce E] \
                 [--home \"globs\"] [--project-to path] [--parser p] [--links \"a,b\"] \
                 [--extensions \"txt,yaml\"] [--title T]";
    match args.first().map(String::as_str) {
        Some("set") => {
            let slug = args.get(1).filter(|s| !s.starts_with("--")).ok_or(usage)?;
            let mut props: Vec<(String, String)> = Vec::new();
            let mut title = None;
            let mut it = args[2..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--applies-to" | "--capture" | "--fields" | "--requires" | "--home"
                    | "--project-to" | "--links" | "--extensions" => {
                        let key = a.trim_start_matches("--").replace('-', "_");
                        let v = it.next().cloned().ok_or(format!("{a} needs a value"))?;
                        props.push((key, v));
                    }
                    "--rot" => {
                        let v = it.next().cloned().ok_or("--rot needs a value")?;
                        if !["none", "info", "warn"].contains(&v.as_str()) {
                            return Err(format!("--rot must be none|info|warn, got '{v}'"));
                        }
                        props.push(("rot".to_string(), v));
                    }
                    "--placement" => {
                        let v = it.next().cloned().ok_or("--placement needs a value")?;
                        if !["graph_first", "file_first", "projection"].contains(&v.as_str()) {
                            return Err(format!(
                                "--placement must be graph_first|file_first|projection, got '{v}'"
                            ));
                        }
                        props.push(("placement".to_string(), v));
                    }
                    "--enforce" => {
                        let v = it.next().cloned().ok_or("--enforce needs a value")?;
                        if !["advisory", "enforced"].contains(&v.as_str()) {
                            return Err(format!("--enforce must be advisory|enforced, got '{v}'"));
                        }
                        props.push(("enforce".to_string(), v));
                    }
                    "--parser" => {
                        let v = it.next().cloned().ok_or("--parser needs a value")?;
                        if !["doc.decision", "doc.plan", "agent", "fields"].contains(&v.as_str()) {
                            return Err(format!(
                                "--parser must be doc.decision|doc.plan|agent|fields, got '{v}'"
                            ));
                        }
                        props.push(("parser".to_string(), v));
                    }
                    "--title" => title = it.next().cloned(),
                    other => return Err(format!("unexpected argument '{other}'\n{usage}")),
                }
            }
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = templates::template_sid(slug);
            let mut labels = std::collections::BTreeMap::new();
            labels.insert("slug".to_string(), slug.to_string());
            labels.insert(
                "title".to_string(),
                title.clone().unwrap_or_else(|| slug.to_string()),
            );
            store
                .put(&Object::Entity {
                    id: sid.clone(),
                    entity_kind: "template".to_string(),
                    labels,
                })
                .map_err(|e| e.to_string())?;
            if let Some(t) = title {
                props.push(("title".to_string(), t));
            }
            let now = now_ms();
            let mut wrote = 0;
            for (prop, value) in &props {
                if twin::latest(&index, &store, &sid, prop)
                    .map_err(|e| e.to_string())?
                    .as_deref()
                    != Some(value.as_str())
                {
                    store
                        .put(&Object::Observation {
                            subject: sid.clone(),
                            property: prop.clone(),
                            value: value.clone(),
                            source: "agent".to_string(),
                            observed_at_ms: now,
                        })
                        .map_err(|e| e.to_string())?;
                    wrote += 1;
                }
            }
            // Version the contract: this-run values win over the index.
            let this_run = |key: &str| {
                props
                    .iter()
                    .rev()
                    .find(|(p, _)| p == key)
                    .map(|(_, v)| v.clone())
            };
            let requires = match this_run("requires") {
                Some(v) => v,
                None => twin::latest(&index, &store, &sid, "requires")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default(),
            };
            let content = twin::latest(&index, &store, &sid, "content")
                .map_err(|e| e.to_string())?
                .unwrap_or_default();
            wrote +=
                templates::stamp_contract(&store, &index, &sid, &requires, &content, "agent", now)
                    .map_err(|e| e.to_string())?;
            println!("template '{slug}': {wrote} observation(s) written");
            if props.iter().any(|(p, _)| p == "capture") {
                println!("the twin now auto-captures matching paths on every refresh");
            }
            Ok(())
        }
        Some("seed") => {
            let store = open_store()?;
            let n = templates::seed(&store).map_err(|e| e.to_string())?;
            println!(
                "{} templates present; {n} observation(s) written",
                templates::DEFAULTS.len()
            );
            Ok(())
        }
        Some("fitness") => {
            let slug = args
                .get(1)
                .filter(|a| !a.starts_with("--"))
                .map(String::as_str);
            let prefix = parse_prefix(&args[1..]);
            let store = open_store()?;
            let index = build_index(&store)?;
            let all = brain_observe::fitness::fitness(&store, &index, &prefix, slug)
                .map_err(|e| e.to_string())?;
            if all.is_empty() {
                println!("no fitness data yet — artifacts record their judging contract as they are captured");
                return Ok(());
            }
            for tf in &all {
                println!(
                    "template {} ({}, {}) — {} version(s) seen in use",
                    tf.slug,
                    tf.kind,
                    tf.enforce,
                    tf.versions.len()
                );
                for v in &tf.versions {
                    let cur = if v.current { ", current" } else { "" };
                    let (ok, total) = v.first_conform;
                    let pct = if total > 0 { ok * 100 / total } else { 100 };
                    println!(
                        "  version {}{cur}: {} artifact(s); first-capture conformance {ok}/{total} ({pct}%)",
                        &v.contract[..v.contract.len().min(12)],
                        v.artifacts
                    );
                    for (field, n) in &v.missing {
                        println!("    missing at first capture: {field} ×{n}");
                    }
                    let outcomes: Vec<String> =
                        v.outcomes.iter().map(|(s, n)| format!("{s} {n}")).collect();
                    println!(
                        "    outcomes: {}; currently stale: {}",
                        outcomes.join(", "),
                        v.stale_now
                    );
                }
                for verdict in &tf.verdicts {
                    println!("  verdict: {verdict}");
                }
            }
            Ok(())
        }
        Some("evolve") => {
            let slug = args.get(1).filter(|a| !a.starts_with("--")).ok_or(usage)?;
            let prefix = parse_prefix(&args[2..]);
            let apply = args.iter().any(|a| a == "--apply");
            let store = open_store()?;
            let index = build_index(&store)?;
            let Some(ev) = brain_observe::fitness::evolve(&store, &index, &prefix, slug)
                .map_err(|e| e.to_string())?
            else {
                println!("no evolution suggested for '{slug}' — the evidence is thin or the contract fits");
                return Ok(());
            };
            println!(
                "proposal for '{slug}': demote [{}] from requires (missed in ≥ half of first captures)",
                ev.demote.join(", ")
            );
            println!(
                "  new: brain template set {slug} --requires \"{}\"  (+ recommended: {})",
                ev.new_requires.join(","),
                ev.new_recommended.join(",")
            );
            if apply {
                brain_observe::fitness::apply_evolution(&store, &index, slug, &ev)
                    .map_err(|e| e.to_string())?;
                println!("applied — contract_b3 bumped; the next measurement window is open");
            } else {
                println!("(re-run with --apply to accept; old artifacts keep the version that judged them)");
            }
            Ok(())
        }
        Some("list") => {
            let store = open_store()?;
            let index = build_index(&store)?;
            for (applies, (sid, requires)) in
                templates::by_kind(&store, &index).map_err(|e| e.to_string())?
            {
                let title = twin::latest(&index, &store, &sid, "title")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default();
                let capture = twin::latest(&index, &store, &sid, "capture")
                    .map_err(|e| e.to_string())?
                    .map(|c| format!("  captures [{c}]"))
                    .unwrap_or_default();
                println!(
                    "{applies:<12} requires [{}]{capture}  — {title}",
                    requires.join(", ")
                );
            }
            Ok(())
        }
        Some("show") => {
            let slug = args.get(1).ok_or(usage)?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = templates::template_sid(slug);
            let content = twin::latest(&index, &store, &sid, "content")
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("no template '{slug}' (run: brain template seed)"))?;
            print!("{content}");
            Ok(())
        }
        _ => Err(usage.to_string()),
    }
}

/// Render every graph-first artifact's projection, plus pure-projection
/// kinds (the capability matrix — a rendered query, never authored).
pub(crate) fn render_projections(
    store: &Store,
    index: &MemIndex,
    root: &std::path::Path,
    prefix: &str,
    only_kind: Option<&str>,
) -> Result<usize, String> {
    use brain_observe::{features, projection, twin};
    let reg = brain_observe::kinds::registry(store, index).map_err(|e| e.to_string())?;
    let mut rendered = 0usize;
    for (kind, def) in &reg {
        if only_kind.is_some_and(|k| k != kind) || def.project_to.is_empty() {
            continue;
        }
        if def.placement == "graph_first" {
            let mut seen = BTreeSet::new();
            for node in index.entities_by_kind(kind) {
                let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
                    continue;
                };
                if labels.get("prefix").map(String::as_str) != Some(prefix)
                    || !seen.insert(id.clone())
                {
                    continue;
                }
                if !brain_observe::lifecycle::of(index, store, &id)
                    .map_err(|e| e.to_string())?
                    .0
                    .is_active()
                {
                    continue;
                }
                let slug = labels.get("slug").cloned().unwrap_or_default();
                let Some(content) =
                    twin::latest(index, store, &id, "content").map_err(|e| e.to_string())?
                else {
                    continue;
                };
                let Some(rel) = projection::projection_rel(def, &slug) else {
                    continue;
                };
                let body = projection::render_body(&rel, kind, prefix, &slug, &content);
                projection::write_projection(store, index, root, &id, &rel, &body)
                    .map_err(|e| e.to_string())?;
                rendered += 1;
            }
        } else if def.placement == "projection" && kind == "capability_matrix" {
            let rows = features::list(store, index, prefix).map_err(|e| e.to_string())?;
            if rows.is_empty() {
                continue;
            }
            let mut content = String::from(
                "# Capability matrix\n\nA rendered query over the feature registry — regenerate, never edit.\n\n| feature | status | definition of done |\n|---|---|---|\n",
            );
            for row in &rows {
                let report = features::evaluate(store, index, prefix, &row.slug)
                    .map_err(|e| e.to_string())?;
                let met = report.checks.iter().filter(|c| c.count > 0).count();
                let mark = if report.done { " ✓" } else { "" };
                content.push_str(&format!(
                    "| {} | {} | {met}/{}{mark} |\n",
                    row.slug,
                    row.status,
                    report.checks.len()
                ));
            }
            let slug = "features";
            let sid = brain_core::ids::StableId::derive(&["capability_matrix", prefix, slug]);
            let mut labels = std::collections::BTreeMap::new();
            labels.insert("prefix".to_string(), prefix.to_string());
            labels.insert("slug".to_string(), slug.to_string());
            labels.insert("title".to_string(), "Capability matrix".to_string());
            store
                .put(&Object::Entity {
                    id: sid.clone(),
                    entity_kind: "capability_matrix".to_string(),
                    labels,
                })
                .map_err(|e| e.to_string())?;
            if twin::latest(index, store, &sid, "content")
                .map_err(|e| e.to_string())?
                .as_deref()
                != Some(content.as_str())
            {
                store
                    .put(&Object::Observation {
                        subject: sid.clone(),
                        property: "content".to_string(),
                        value: content.clone(),
                        source: "projection".to_string(),
                        observed_at_ms: now_ms(),
                    })
                    .map_err(|e| e.to_string())?;
            }
            let Some(rel) = projection::projection_rel(def, slug) else {
                continue;
            };
            let body = projection::render_body(&rel, kind, prefix, slug, &content);
            projection::write_projection(store, index, root, &sid, &rel, &body)
                .map_err(|e| e.to_string())?;
            rendered += 1;
        }
    }
    Ok(rendered)
}

/// `brain artifact ...` — generic browse for artifacts of any entity kind,
/// including kinds taught to the store via graph-defined capture rules.
pub(crate) fn cmd_artifact(args: &[String]) -> Result<(), String> {
    let usage = "usage: brain artifact new|edit <prefix> <kind> <slug> [--title T] [--file f|-] | \
                 brain artifact render [dir] [--prefix <p>] [--kind k] [--check] | \
                 brain artifact list <prefix> <kind> [--all] | \
                 brain artifact show <prefix> <kind> <slug> | \
                 brain artifact set-lifecycle <prefix> <kind> <slug> <state> [--why R] | \
                 brain artifact ack <prefix> <kind> <slug> [--note T]";
    match args.first().map(String::as_str) {
        Some(op @ ("new" | "edit")) => {
            let (prefix, kind, slug) = match (args.get(1), args.get(2), args.get(3)) {
                (Some(p), Some(k), Some(s))
                    if !p.starts_with("--") && !k.starts_with("--") && !s.starts_with("--") =>
                {
                    (p, k, s)
                }
                _ => return Err(usage.to_string()),
            };
            let mut title = None;
            let mut file = None;
            let mut it = args[4..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--title" => title = it.next().cloned(),
                    "--file" => file = it.next().cloned(),
                    other => return Err(format!("unexpected argument '{other}'\n{usage}")),
                }
            }
            let store = open_store()?;
            let index = build_index(&store)?;
            let reg = brain_observe::kinds::registry(&store, &index).map_err(|e| e.to_string())?;
            let def = reg
                .get(kind.as_str())
                .ok_or_else(|| format!("unknown kind '{kind}' — teach it: brain template set <slug> --applies-to {kind} ..."))?
                .clone();
            let sid = brain_core::ids::StableId::derive(&[kind, prefix, slug]);
            let exists = !index.entity_nodes(&sid).is_empty();
            if op == "edit" && !exists {
                return Err(format!(
                    "no {kind} '{slug}' under {prefix} — use artifact new"
                ));
            }
            let title = title.unwrap_or_else(|| slug.replace('-', " "));
            let content = match file.as_deref() {
                Some("-") => {
                    use std::io::Read as _;
                    let mut buf = String::new();
                    std::io::stdin()
                        .read_to_string(&mut buf)
                        .map_err(|e| e.to_string())?;
                    buf
                }
                Some(path) => std::fs::read_to_string(path)
                    .map_err(|e| format!("cannot read '{path}': {e}"))?,
                None if op == "new" && !def.content.is_empty() => {
                    brain_observe::templates::instantiate(&def.content, &title)
                }
                None => return Err(format!("--file <path|-> is required\n{usage}")),
            };

            // The write-time gate: validate before anything lands.
            let missing = brain_observe::templates::check(&content, &def.requires);
            if !missing.is_empty() {
                let fix = format!(
                    "missing: {} — scaffold: brain deliverable new {} --title \"{title}\"",
                    missing.join(", "),
                    def.slug
                );
                if def.enforce == "enforced" {
                    return Err(format!(
                        "refused: {kind} '{slug}' does not meet its contract; nothing written\n  {fix}"
                    ));
                }
                eprintln!(
                    "warning: {kind} '{slug}' does not meet its contract ({fix}) — recorded anyway"
                );
            }
            let out = brain_observe::twin::author_artifact(
                &store, prefix, kind, slug, &title, &content, "agent",
            )
            .map_err(|e| e.to_string())?;
            let state = if out.wrote {
                "recorded"
            } else {
                "already recorded (unchanged)"
            };
            println!(
                "{kind} '{slug}' {state} under {prefix} ({} mention(s))",
                out.mentions.len()
            );

            // Graph-first kinds materialize as read-only projections.
            if def.placement == "graph_first" {
                if let Some(rel) = brain_observe::projection::projection_rel(&def, slug) {
                    let index = build_index(&store)?;
                    let body =
                        brain_observe::projection::render_body(&rel, kind, prefix, slug, &content);
                    let target = brain_observe::projection::write_projection(
                        &store,
                        &index,
                        std::path::Path::new("."),
                        &out.sid,
                        &rel,
                        &body,
                    )
                    .map_err(|e| e.to_string())?;
                    println!("projected (read-only): {}", target.display());
                }
            }
            Ok(())
        }
        Some("render") => {
            let dir = args
                .get(1)
                .filter(|a| !a.starts_with("--"))
                .cloned()
                .unwrap_or_else(|| ".".to_string());
            let prefix = parse_prefix(&args[1..]);
            let only_kind = args
                .iter()
                .position(|a| a == "--kind")
                .and_then(|i| args.get(i + 1))
                .cloned();
            let check = args.iter().any(|a| a == "--check");
            let store = open_store()?;
            let index = build_index(&store)?;
            let root = std::path::Path::new(&dir);
            if check {
                let found = brain_observe::projection::drift(&store, &index, root, &prefix)
                    .map_err(|e| e.to_string())?;
                if found.is_empty() {
                    println!("all projections match their contracts");
                    return Ok(());
                }
                for d in &found {
                    println!("{:?} {} — fix: {}", d.kind, d.path, d.fix);
                }
                if found
                    .iter()
                    .any(|d| d.kind == brain_observe::projection::DriftKind::HandEdited)
                {
                    return Err(format!(
                        "refused: {} projection(s) drifted from the graph",
                        found.len()
                    ));
                }
                return Ok(());
            }
            let rendered = render_projections(&store, &index, root, &prefix, only_kind.as_deref())?;
            println!(
                "{rendered} projection(s) rendered under {}/docs/brain",
                dir.trim_end_matches('/')
            );
            Ok(())
        }
        Some("ack") => {
            let (prefix, kind, slug) = match (args.get(1), args.get(2), args.get(3)) {
                (Some(p), Some(k), Some(s)) => (p, k, s),
                _ => return Err(usage.to_string()),
            };
            let note = args
                .iter()
                .position(|a| a == "--note")
                .and_then(|i| args.get(i + 1))
                .map(String::as_str)
                .unwrap_or("reviewed, still accurate");
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = brain_core::ids::StableId::derive(&[kind, prefix, slug]);
            if index.entity_nodes(&sid).is_empty() {
                return Err(format!("no {kind} '{slug}' under {prefix}"));
            }
            brain_observe::twin::ack(&store, &sid, note).map_err(|e| e.to_string())?;
            println!("{kind} '{slug}' acknowledged — staleness clock reset");
            Ok(())
        }
        Some("set-lifecycle") => {
            let (prefix, kind, slug, state) =
                match (args.get(1), args.get(2), args.get(3), args.get(4)) {
                    (Some(p), Some(k), Some(s), Some(st)) => (p, k, s, st),
                    _ => return Err(usage.to_string()),
                };
            let state = brain_observe::lifecycle::Lifecycle::parse(state).ok_or_else(|| {
                format!("unknown state '{state}' (active|done|abandoned|retired|superseded)")
            })?;
            let why = args
                .iter()
                .position(|a| a == "--why")
                .and_then(|i| args.get(i + 1))
                .map(String::as_str);
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = brain_core::ids::StableId::derive(&[kind, prefix, slug]);
            if index.entity_nodes(&sid).is_empty() {
                return Err(format!("no {kind} '{slug}' under {prefix}"));
            }
            let wrote = brain_observe::lifecycle::set(&store, &index, &sid, state, why)
                .map_err(|e| e.to_string())?;
            let verb = if wrote { "now" } else { "already" };
            println!("{kind} '{slug}' {verb} {}", state.as_str());
            Ok(())
        }
        Some("list") => {
            let (prefix, kind) = match (args.get(1), args.get(2)) {
                (Some(p), Some(k)) => (p, k),
                _ => return Err(usage.to_string()),
            };
            let all = args.iter().any(|a| a == "--all");
            let store = open_store()?;
            let index = build_index(&store)?;
            let mut seen = BTreeSet::new();
            let mut any = false;
            let mut hidden = 0usize;
            for node in index.entities_by_kind(kind) {
                let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
                    continue;
                };
                if labels.get("prefix") != Some(prefix) || !seen.insert(id.clone()) {
                    continue;
                }
                let (lc, _) =
                    brain_observe::lifecycle::of(&index, &store, &id).map_err(|e| e.to_string())?;
                if !lc.is_active() && !all {
                    hidden += 1;
                    continue;
                }
                let slug = labels.get("slug").cloned().unwrap_or_default();
                let title = brain_observe::twin::latest(&index, &store, &id, "title")
                    .map_err(|e| e.to_string())?
                    .or_else(|| labels.get("title").cloned())
                    .unwrap_or_else(|| slug.clone());
                let conforms = brain_observe::twin::latest(&index, &store, &id, "conforms")
                    .map_err(|e| e.to_string())?
                    .map(|c| {
                        if c == "true" {
                            "".to_string()
                        } else {
                            "  [nonconforming]".to_string()
                        }
                    })
                    .unwrap_or_default();
                let mentions = relation_targets(&store, &index, &id, "mentions")?.len();
                let tag = if lc.is_active() {
                    String::new()
                } else {
                    format!("  [{}]", lc.as_str())
                };
                println!("{slug}: {title}  ({mentions} mention(s)){conforms}{tag}");
                any = true;
            }
            if !any {
                println!(
                    "no {}{kind} artifacts under {prefix}",
                    if all { "" } else { "active " }
                );
            }
            if hidden > 0 {
                println!("({hidden} non-active hidden — --all shows history)");
            }
            Ok(())
        }
        Some("show") => {
            let (prefix, kind, slug) = match (args.get(1), args.get(2), args.get(3)) {
                (Some(p), Some(k), Some(s)) => (p, k, s),
                _ => return Err(usage.to_string()),
            };
            let store = open_store()?;
            let index = build_index(&store)?;
            let sid = brain_core::ids::StableId::derive(&[kind, prefix, slug]);
            // Latest value per property, so extracted fields show as a header.
            let mut latest_props: std::collections::BTreeMap<String, (u64, String)> =
                Default::default();
            for id in index.observations_of(&sid) {
                if let Object::Observation {
                    property,
                    value,
                    observed_at_ms,
                    ..
                } = store.get(&id).map_err(|e| e.to_string())?
                {
                    let entry = latest_props.entry(property).or_insert((0, String::new()));
                    if observed_at_ms >= entry.0 {
                        *entry = (observed_at_ms, value);
                    }
                }
            }
            if latest_props.is_empty() {
                return Err(format!("no {kind} '{slug}' under {prefix}"));
            }
            for (prop, (_, value)) in &latest_props {
                if prop != "content" {
                    println!("{prop}: {value}");
                }
            }
            if let Some((_, content)) = latest_props.get("content") {
                println!("---");
                print!("{content}");
                if !content.ends_with('\n') {
                    println!();
                }
            }
            let mentions = relation_targets(&store, &index, &sid, "mentions")?;
            if !mentions.is_empty() {
                println!("--- mentions ---");
                for m in mentions {
                    println!("{}", entity_label(&store, &index, &m));
                }
            }
            Ok(())
        }
        _ => Err(usage.to_string()),
    }
}

/// `brain asset ...` — typed binary artifacts: bytes stay files, identity,
/// ownership, and staleness live in the graph.
pub(crate) fn cmd_asset(args: &[String]) -> Result<(), String> {
    let usage = "usage: brain asset add <file> --prefix <p> --for <kind>/<slug> \
                 [--depicts <path|kind/slug>]... [--subtype s] | brain asset list <prefix> [--all]";
    match args.first().map(String::as_str) {
        Some("add") => {
            let file = args.get(1).filter(|a| !a.starts_with("--")).ok_or(usage)?;
            let mut prefix = None;
            let mut owner = None;
            let mut subtype = None;
            let mut depicts: Vec<String> = Vec::new();
            let mut it = args[2..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--prefix" => prefix = it.next().cloned(),
                    "--for" => owner = it.next().cloned(),
                    "--subtype" => subtype = it.next().cloned(),
                    "--depicts" => {
                        depicts.push(it.next().cloned().ok_or("--depicts needs a value")?)
                    }
                    other => return Err(format!("unexpected argument '{other}'\n{usage}")),
                }
            }
            let prefix = prefix.ok_or_else(|| format!("--prefix is required\n{usage}"))?;
            let owner = owner.ok_or_else(|| format!("--for <kind>/<slug> is required\n{usage}"))?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let rel = file.trim_start_matches("./");
            if store
                .resolve(&format!("{prefix}/{rel}"))
                .map_err(|e| e.to_string())?
                .is_none()
            {
                return Err(format!(
                    "'{rel}' is not twinned under {prefix} — run `brain twin refresh` first"
                ));
            }
            let (okind, oslug) = owner
                .split_once('/')
                .ok_or("--for takes <kind>/<slug>, e.g. plan/twin-v3")?;
            let owner_sid = brain_core::ids::StableId::derive(&[okind, &prefix, oslug]);
            if index.entity_nodes(&owner_sid).is_empty() {
                return Err(format!("no {okind} '{oslug}' under {prefix}"));
            }
            let mut targets = Vec::new();
            for d in &depicts {
                let sid = brain_observe::assets::resolve_depicts(&store, &index, &prefix, d)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("--depicts '{d}' matches no twinned entity"))?;
                targets.push(sid);
            }
            let out = brain_observe::assets::add(
                &store,
                &prefix,
                rel,
                &owner_sid,
                &targets,
                subtype.as_deref(),
            )
            .map_err(|e| e.to_string())?;
            let state = if out.wrote {
                "declared"
            } else {
                "already declared (unchanged)"
            };
            println!(
                "asset '{}' {state} under {prefix} (owner: {owner}, {} depicts)",
                out.slug,
                targets.len()
            );
            Ok(())
        }
        Some("list") => {
            let prefix = args.get(1).filter(|a| !a.starts_with("--")).ok_or(usage)?;
            let all = args.iter().any(|a| a == "--all");
            let store = open_store()?;
            let index = build_index(&store)?;
            let rows =
                brain_observe::assets::list(&store, &index, prefix).map_err(|e| e.to_string())?;
            let mut any = false;
            let mut hidden = 0usize;
            for row in rows {
                if !row.lifecycle.is_active() && !all {
                    hidden += 1;
                    continue;
                }
                let owner = row.owner.map(|o| format!("  -> {o}")).unwrap_or_default();
                let tag = if row.lifecycle.is_active() {
                    String::new()
                } else {
                    format!("  [{}]", row.lifecycle.as_str())
                };
                println!("[{}] {} ({}){owner}{tag}", row.subtype, row.slug, row.path);
                any = true;
            }
            if !any {
                println!(
                    "no {}assets under {prefix}",
                    if all { "" } else { "active " }
                );
            }
            if hidden > 0 {
                println!("({hidden} non-active hidden — --all shows history)");
            }
            Ok(())
        }
        _ => Err(usage.to_string()),
    }
}

/// `brain deliverable new <template>` — instantiate a scaffold to stdout.
pub(crate) fn cmd_deliverable(args: &[String]) -> Result<(), String> {
    use brain_observe::{templates, twin};
    let usage = "usage: brain deliverable new <template-slug> [--title T]";
    if args.first().map(String::as_str) != Some("new") {
        return Err(usage.to_string());
    }
    let slug = args.get(1).ok_or(usage)?;
    let mut title = "Untitled".to_string();
    let mut it = args[2..].iter();
    while let Some(a) = it.next() {
        if a == "--title" {
            if let Some(t) = it.next() {
                title = t.clone();
            }
        }
    }
    let store = open_store()?;
    let index = build_index(&store)?;
    let sid = templates::template_sid(slug);
    let content = twin::latest(&index, &store, &sid, "content")
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no template '{slug}' (run: brain template seed)"))?;
    print!("{}", templates::instantiate(&content, &title));
    Ok(())
}
