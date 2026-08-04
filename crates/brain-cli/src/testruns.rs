//! Test protocols: import a run, list what landed, retire what no code declares.

use brain_store::now_ms;
use crate::support::*;

/// `brain testrun ...` — test protocols in the graph.
pub(crate) fn cmd_testrun(args: &[String]) -> Result<(), String> {
    use brain_observe::testing;
    let usage = "usage: brain testrun import <report-file|-> --prefix <p> [--dir <d>] | brain testrun list <prefix> | brain testrun purge <prefix> [--dry-run]";
    match args.first().map(String::as_str) {
        Some("import") => {
            let mut file = None;
            let mut prefix = None;
            let mut dir = ".".to_string();
            let mut it = args[1..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--prefix" => prefix = it.next().cloned(),
                    "--dir" => dir = it.next().cloned().unwrap_or_else(|| ".".to_string()),
                    other if file.is_none() => file = Some(other.to_string()),
                    other => return Err(format!("unexpected argument '{other}'\n{usage}")),
                }
            }
            let file = file.ok_or(usage)?;
            let prefix = prefix.ok_or_else(|| format!("--prefix is required\n{usage}"))?;
            let raw = if file == "-" {
                use std::io::Read as _;
                let mut buf = String::new();
                std::io::stdin()
                    .read_to_string(&mut buf)
                    .map_err(|e| e.to_string())?;
                buf
            } else {
                std::fs::read_to_string(&file).map_err(|e| format!("cannot read '{file}': {e}"))?
            };
            let report = testing::parse_report(&raw);
            if report.cases.is_empty() {
                return Err(
                    "no test cases recognized (expected cargo-test output, JUnit XML, or Playwright JSON)"
                        .to_string(),
                );
            }
            let store = open_store()?;
            let out = testing::record_run_in(
                &store,
                std::path::Path::new(&dir),
                &prefix,
                &report,
                &raw,
            )
            .map_err(|e| e.to_string())?;
            let state = if out.wrote {
                "recorded"
            } else {
                "already imported (unchanged)"
            };
            println!(
                "run {state}: {} total, {} passed, {} failed, {} skipped ({}); {} transition(s)",
                out.total, out.passed, out.failed, out.skipped, report.format, out.transitions
            );
            for name in &out.failing {
                println!("  FAILED {name}");
            }
            let attachments: usize = report.cases.iter().map(|c| c.attachments.len()).sum();
            if attachments > 0 {
                println!(
                    "  {attachments} attachment(s) linked to their cases — run `brain twin refresh {dir} --prefix {prefix}` to hash them"
                );
            }
            Ok(())
        }
        Some("list") => {
            let prefix = args.get(1).ok_or(usage)?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let now = now_ms();
            let runs = testing::runs(&store, &index, prefix).map_err(|e| e.to_string())?;
            if runs.is_empty() {
                println!("no test runs under {prefix}");
            }
            for (at, total, passed, failed, format) in runs {
                let age = now.saturating_sub(at) / 1000;
                let verdict = if failed == 0 { "ok" } else { "FAILED" };
                println!("[{age:>6}s ago] {verdict}: {passed}/{total} passed, {failed} failed ({format})");
            }
            Ok(())
        }
        // A test that was renamed or deleted keeps answering for code
        // that no longer exists. This retires those — as a recorded
        // fact, not a deletion: every past result stays readable, and a
        // run that names the test again brings it back.
        Some("purge") => {
            let mut prefix = None;
            let mut dry = false;
            for arg in &args[1..] {
                match arg.as_str() {
                    "--dry-run" => dry = true,
                    other if prefix.is_none() => prefix = Some(other.to_string()),
                    other => return Err(format!("unexpected argument '{other}'\n{usage}")),
                }
            }
            let prefix = prefix.ok_or(usage)?;
            let store = open_store()?;
            let index = build_index(&store)?;
            let unseen =
                testing::unseen_cases(&store, &index, &prefix).map_err(|e| e.to_string())?;
            if unseen.is_empty() {
                println!("every recorded case is still declared by code the twin can see");
                return Ok(());
            }
            for (_, name, _) in &unseen {
                println!("  {name}");
            }
            if dry {
                println!(
                    "{} case(s) no code declares any more; run without --dry-run to retire them",
                    unseen.len()
                );
                return Ok(());
            }
            let retired = testing::purge_unseen(&store, &index, &prefix, now_ms())
                .map_err(|e| e.to_string())?;
            println!(
                "retired {} case(s) under {prefix} — history kept, readers stop counting them",
                retired.len()
            );
            Ok(())
        }
        _ => Err(usage.to_string()),
    }
}
