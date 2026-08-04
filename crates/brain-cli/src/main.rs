//! `brain` — the command-line surface of the substrate.
//!
//! The CLI is a projection instrument: it renders and drives the graph, but
//! holds no state of its own. All state lives in the store (default `.brain/`).


mod automation;
mod docsgen;
mod documents;
mod features;
mod functions;
mod govern;
mod hooks;
mod manual;
mod native;
mod notation;
mod sessions;
mod store;
mod support;
mod tasks;
mod testruns;
mod twin;
mod util;

use crate::support::*;
use std::process::ExitCode;

fn usage() -> String {
    manual::usage_text()
}

fn main() -> ExitCode {
    // Die quietly on a closed pipe (`brain status | head`) instead of
    // panicking — restore the default Unix SIGPIPE behavior Rust masks.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("init") => store::cmd_init(),
        Some("status") => store::cmd_status(),
        Some("names") => store::cmd_names(),
        Some("put-code") => native::cmd_put_code(&args[1..]),
        Some("notation") => native::cmd_notation(&args[1..]),
        Some("run") => native::cmd_run(&args[1..]),
        Some("recover") => govern::cmd_recover(),
        Some("ingest") => twin::cmd_twin_refresh(&args[1..], true),
        Some("twin") => twin::cmd_twin(&args[1..]),
        Some("note") => twin::cmd_note(&args[1..]),
        Some("notes") => twin::cmd_notes(&args[1..]),
        Some("adr") => documents::cmd_doc(&args[1..], brain_observe::docs::DocKind::Decision),
        Some("plan") => documents::cmd_doc(&args[1..], brain_observe::docs::DocKind::Plan),
        Some("skill") => documents::cmd_agent_doc(&args[1..], brain_observe::agents::AgentDocKind::Skill),
        Some("agentcfg") => documents::cmd_agent_doc(&args[1..], brain_observe::agents::AgentDocKind::Config),
        Some("template") => documents::cmd_template(&args[1..]),
        Some("artifact") => documents::cmd_artifact(&args[1..]),
        Some("asset") => documents::cmd_asset(&args[1..]),
        Some("instructions") => automation::cmd_instructions(&args[1..]),
        Some("tidy") => functions::cmd_tidy(&args[1..]),
        Some("deliverable") => documents::cmd_deliverable(&args[1..]),
        Some("feature") => features::cmd_feature(&args[1..]),
        Some("baseline") => twin::cmd_baseline(&args[1..]),
        Some("done") => features::cmd_done(&args[1..]),
        Some("testrun") => testruns::cmd_testrun(&args[1..]),
        Some("sessions") => sessions::cmd_sessions(&args[1..]),
        Some("change") => govern::cmd_change(&args[1..]),
        Some("bench") => util::cmd_bench(&args[1..]),
        Some("relation") => twin::cmd_relation(&args[1..]),
        Some("wake") => functions::cmd_wake(&args[1..]),
        Some("attend") => functions::cmd_attend(&args[1..]),
        Some("spine") => functions::cmd_spine(&args[1..]),
        Some("sleep") => functions::cmd_sleep(&args[1..]),
        Some("related") => functions::cmd_related(&args[1..]),
        Some("before") => functions::cmd_before(&args[1..]),
        Some("next") => functions::cmd_next(&args[1..]),
        Some("find") => functions::cmd_find(&args[1..]),
        Some("can-i") => functions::cmd_can_i(&args[1..]),
        Some("eyes") => functions::cmd_eyes(&args[1..]),
        Some("docs") => docsgen::cmd_docs(&args[1..], open_store),
        Some("hook") => hooks::cmd_hook(&args[1..], open_store),
        Some("watch") => automation::cmd_watch(&args[1..]),
        Some("man") => util::cmd_man(&args[1..]),
        Some("version") | Some("--version") | Some("-V") => {
            println!("brain {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("pull") => store::cmd_sync(&args[1..], true),
        Some("push") => store::cmd_sync(&args[1..], false),
        Some("refs") => util::cmd_refs(&args[1..]),
        Some("evidence") => util::cmd_evidence(&args[1..]),
        Some("deps") => util::cmd_deps(&args[1..]),
        Some("observations") => twin::cmd_observations(&args[1..]),
        Some("task") => native::cmd_task(&args[1..]),
        Some("demo") => native::cmd_demo(),
        _ => {
            eprint!("{}", usage());
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        // Deliberate gate refusals exit 3 so hooks can distinguish "brain
        // refused" (block the commit) from "brain broke" (fail open).
        Err(e) if e.starts_with("refused:") => {
            eprintln!("{e}");
            ExitCode::from(3)
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
