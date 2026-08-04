//! Automation: the watcher and the agent instructions that keep the graph fresh.

use crate::support::*;

/// `brain instructions generate` — one guardrail block, every agent file.
pub(crate) fn cmd_instructions(args: &[String]) -> Result<(), String> {
    let usage = "usage: brain instructions generate [dir] [--prefix <p>] [--check]";
    if args.first().map(String::as_str) != Some("generate") {
        return Err(usage.to_string());
    }
    let dir = args
        .get(1)
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| ".".to_string());
    let prefix = parse_prefix(&args[1..]);
    let check = args.iter().any(|a| a == "--check");
    let store = open_store()?;
    let index = build_index(&store)?;
    let root = std::path::Path::new(&dir);
    if check {
        let drifted = brain_observe::instructions::block_drift(&store, &index, root, &prefix)
            .map_err(|e| e.to_string())?;
        if drifted.is_empty() {
            println!("instruction blocks match the registry");
            return Ok(());
        }
        return Err(format!(
            "instruction block out of date in: {} — run `brain instructions generate {dir} --prefix {prefix}`",
            drifted.join(", ")
        ));
    }
    for (file, changed) in brain_observe::instructions::generate(&store, &index, root, &prefix)
        .map_err(|e| e.to_string())?
    {
        let state = if changed { "updated" } else { "unchanged" };
        println!("{file}: guardrail block {state}");
    }
    println!("every agent family now reads identical rules; regenerate after `brain template set`");
    Ok(())
}

/// `brain watch` — the continuous loop, built in: refresh + insights on an
/// interval, optionally regenerating docs each round. Replaces the shell
/// wrapper so the monolithic binary needs no scripts.
pub(crate) fn cmd_watch(args: &[String]) -> Result<(), String> {
    let mut dir = ".".to_string();
    let mut prefix = "twin/self".to_string();
    let mut interval = 60u64;
    let mut docs = false;
    let mut it = args.iter();
    let mut positional = false;
    while let Some(a) = it.next() {
        match a.as_str() {
            "--prefix" => prefix = it.next().cloned().ok_or("--prefix needs a value")?,
            "--interval" => {
                interval = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--interval needs seconds")?
            }
            "--docs" => docs = true,
            other if !other.starts_with("--") && !positional => {
                dir = other.to_string();
                positional = true;
            }
            other => return Err(format!("unexpected argument '{other}'")),
        }
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    loop {
        println!("--- watch: refresh {prefix} ---");
        for cmd in [
            vec!["twin", "refresh", dir.as_str(), "--prefix", prefix.as_str()],
            vec!["twin", "insights", prefix.as_str()],
        ] {
            let _ = std::process::Command::new(&exe).args(&cmd).status();
        }
        if docs {
            let _ = std::process::Command::new(&exe)
                .args([
                    "docs",
                    "generate",
                    dir.as_str(),
                    "--prefix",
                    prefix.as_str(),
                ])
                .status();
        }
        std::thread::sleep(std::time::Duration::from_secs(interval));
    }
}
