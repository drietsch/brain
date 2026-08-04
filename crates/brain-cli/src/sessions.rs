//! Coding-agent sessions: who worked here, and on what.

use brain_store::now_ms;
use crate::support::*;

/// `brain sessions ...` — the coding agents that worked here.
pub(crate) fn cmd_sessions(args: &[String]) -> Result<(), String> {
    use brain_observe::sessions;
    let usage = "usage: brain sessions import [dir] [--prefix <p>] [--agent claude|codex] [--since <ms|30m|2h|7d>] | brain sessions list <prefix> [--json] | brain sessions annotate <prefix> <session-id> [--objective <text>] [--outcome shipped|abandoned|superseded]";
    match args.first().map(String::as_str) {
        Some("import") => {
            let mut dir = ".".to_string();
            let mut prefix = "twin/self".to_string();
            let mut agent: Option<String> = None;
            let mut since = 0u64;
            let mut it = args[1..].iter();
            let mut positional_taken = false;
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--prefix" => prefix = it.next().cloned().unwrap_or(prefix),
                    "--agent" => agent = it.next().cloned(),
                    "--since" => {
                        let raw = it.next().cloned().unwrap_or_default();
                        since = parse_since(&raw)?;
                    }
                    other if !other.starts_with("--") && !positional_taken => {
                        dir = other.to_string();
                        positional_taken = true;
                    }
                    other => return Err(format!("unexpected argument '{other}'\n{usage}")),
                }
            }
            if let Some(agent) = agent.as_deref() {
                if !matches!(agent, "claude" | "codex") {
                    return Err(format!("unknown agent '{agent}' (claude|codex)\n{usage}"));
                }
            }
            let home = home_dir().ok_or("cannot locate the home directory")?;
            let store = open_store()?;
            let out = sessions::import(
                &store,
                &home,
                std::path::Path::new(&dir),
                &prefix,
                agent.as_deref(),
                since,
            )
            .map_err(|e| e.to_string())?;
            println!(
                "sessions: {} imported, {} unchanged, {} ran in another workspace",
                out.imported, out.unchanged, out.elsewhere
            );
            Ok(())
        }
        Some("annotate") => {
            let pos = positional(&args[1..]);
            let (prefix, id) = match pos.as_slice() {
                [p, i] => (*p, *i),
                _ => return Err(usage.to_string()),
            };
            let flag = |key: &str| -> Option<String> {
                args.iter()
                    .position(|a| a == key)
                    .and_then(|i| args.get(i + 1).cloned())
            };
            let objective = flag("--objective");
            let outcome = flag("--outcome");
            if objective.is_none() && outcome.is_none() {
                return Err(format!("nothing to annotate\n{usage}"));
            }
            if let Some(o) = outcome.as_deref() {
                if !sessions::OUTCOMES.contains(&o) {
                    return Err(format!(
                        "unknown outcome '{o}' ({})",
                        sessions::OUTCOMES.join("|")
                    ));
                }
            }
            let store = open_store()?;
            let index = build_index(&store)?;
            let rows = sessions::list(&store, &index, prefix).map_err(|e| e.to_string())?;
            let matched: Vec<&sessions::SessionRow> =
                rows.iter().filter(|r| r.id.starts_with(id)).collect();
            let full_id = match matched.as_slice() {
                [one] => one.id.clone(),
                [] => return Err(format!("no session under {prefix} with id starting '{id}'")),
                many => {
                    return Err(format!(
                        "{} sessions match '{id}' — be more specific",
                        many.len()
                    ))
                }
            };
            sessions::annotate(
                &store,
                prefix,
                &full_id,
                objective.as_deref(),
                outcome.as_deref(),
            )
            .map_err(|e| e.to_string())?;
            println!("annotated session {full_id}");
            Ok(())
        }
        Some("list") => {
            let prefix = args.get(1).ok_or(usage)?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let now = now_ms();
            let rows = sessions::list(&store, &index, prefix).map_err(|e| e.to_string())?;
            if wants_json(args) {
                println!("{}", serde_json::to_string(&rows).map_err(|e| e.to_string())?);
                return Ok(());
            }
            if rows.is_empty() {
                println!("no agent sessions under {prefix} (try: brain sessions import)");
                return Ok(());
            }
            for row in rows {
                let ago = now.saturating_sub(row.ended_at_ms) / 1000;
                let minutes = row.ended_at_ms.saturating_sub(row.started_at_ms) / 60_000;
                let short: String = row.id.chars().take(8).collect();
                let outcome = row
                    .outcome
                    .as_deref()
                    .map(|o| format!(" [{o}]"))
                    .unwrap_or_default();
                println!(
                    "[{ago:>6}s ago] {} ({}) {}min, {} turn(s), {} file(s) #{short}{outcome}: {}",
                    row.agent,
                    row.model.as_deref().unwrap_or("model unrecorded"),
                    minutes,
                    row.turns,
                    row.files_touched,
                    row.objective
                );
                if !row.tools.is_empty() {
                    println!("           tools: {}", row.tools);
                }
            }
            Ok(())
        }
        _ => Err(usage.to_string()),
    }
}

/// `30m`, `2h`, `7d`, or raw epoch milliseconds.
pub(crate) fn parse_since(raw: &str) -> Result<u64, String> {
    if raw.is_empty() {
        return Ok(0);
    }
    let now = now_ms();
    let (value, unit) = raw.split_at(raw.len() - 1);
    let scale = match unit {
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => {
            return raw
                .parse::<u64>()
                .map_err(|_| format!("cannot read '{raw}' as a time (try 30m, 2h, 7d, or epoch ms)"))
        }
    };
    let value: u64 = value
        .parse()
        .map_err(|_| format!("cannot read '{raw}' as a time (try 30m, 2h, 7d)"))?;
    Ok(now.saturating_sub(value * scale))
}

pub(crate) fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}
