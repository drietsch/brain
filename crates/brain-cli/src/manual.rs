//! The manual — one command registry, two projections.
//!
//! Every command is described exactly once in [`COMMANDS`]; both the
//! CLI's `usage()` text and the troff man page render from it, so help
//! and `man brain` cannot diverge (ADR-005's rule applied to the tool
//! itself: documentation is a projection, never a second source).

use std::fmt::Write as _;

pub struct Cmd {
    pub group: &'static str,
    pub name: &'static str,
    pub args: &'static str,
    pub summary: &'static str,
}

/// (group title, one-paragraph description) — render order.
pub const GROUPS: &[(&str, &str)] = &[
    (
        "Store",
        "The system of record: a content-addressed graph in .brain/ — immutable objects, an append-only event log, namespace lineage as version control.",
    ),
    (
        "Twin",
        "Reflective mode: a persistent, queryable semantic model of existing software. Observations are sourced and timestamped; only drift is recorded.",
    ),
    (
        "Documents",
        "The why and the contract: decisions (ADRs), plans, skills, agent configuration, templates with conformance, and artifacts of graph-taught kinds.",
    ),
    (
        "Tests",
        "Tests as graph citizens: framework classification and covers-relations at refresh; imported run protocols with per-case result timelines.",
    ),
    (
        "Sessions",
        "Who worked here: the coding-agent sessions that ran in this workspace, with their objective, model, tools and blast radius — the graph's only record of a principal.",
    ),
    (
        "Features",
        "The registry: features as entities, the definition of done as relation predicates, done-ness as a query.",
    ),
    (
        "Brain functions",
        "The functional organs: attention (what matters now), consolidation (distill history into memory), association (what is related, and why).",
    ),
    (
        "Governed changes",
        "The motor system: mutations to twinned software through the durable intent/receipt boundary, with explicit capability and full provenance.",
    ),
    (
        "Automation",
        "Git triggers the brain: fail-open hooks on commit and push, an in-binary watch loop, and regenerated documentation projections.",
    ),
    (
        "Native code",
        "The substrate's own calculus: content-addressed programs, capability-gated effects, evidence-cached task checking.",
    ),
    (
        "Utilities",
        "Replication, graph inspection, recovery, and the manual itself.",
    ),
];

pub const COMMANDS: &[Cmd] = &[
    Cmd { group: "Store", name: "init", args: "", summary: "create a store in ./.brain and seed the deliverable templates" },
    Cmd { group: "Store", name: "status", args: "", summary: "objects, namespace size, HEAD, intent states" },
    Cmd { group: "Store", name: "names", args: "", summary: "list name -> node bindings" },
    Cmd { group: "Store", name: "pull", args: "<store-root>", summary: "replicate another store into this one (set union, conflicts preserved)" },
    Cmd { group: "Store", name: "push", args: "<store-root>", summary: "replicate this store into another" },

    Cmd { group: "Twin", name: "twin refresh", args: "<dir> [--prefix <p>] [--full] [--json]", summary: "observe a source tree, record only drift; --full reprocesses every file after extractor upgrades" },
    Cmd { group: "Twin", name: "twin status", args: "<dir> [--prefix <p>] [--json]", summary: "the same comparison, read-only" },
    Cmd { group: "Twin", name: "twin backfill", args: "<dir> [--prefix <p>] [--max-commits N]", summary: "replay git history into the twin with historical timestamps (brownfield minute-one)" },
    Cmd { group: "Twin", name: "twin files", args: "<prefix>", summary: "twinned files with language, symbols, freshness" },
    Cmd { group: "Twin", name: "twin symbols", args: "<name> [--json]", summary: "what a file declares, with line numbers" },
    Cmd { group: "Twin", name: "twin imports", args: "<name> [--transitive]", summary: "what a file depends on; --transitive walks the closure" },
    Cmd { group: "Twin", name: "twin rdeps", args: "<name> [--transitive] [--json]", summary: "who depends on a file; --transitive is the blast radius" },
    Cmd { group: "Twin", name: "twin at", args: "<prefix> <ms|30m|2h|1d|git-hash|baseline>", summary: "the twin as it was: bi-temporal read at a moment, commit, or baseline" },
    Cmd { group: "Twin", name: "baseline add", args: "<prefix> <name> [--at <when>]", summary: "name a moment so it can be asked about later" },
    Cmd { group: "Twin", name: "baseline list", args: "<prefix>", summary: "every named moment, newest first" },
    Cmd { group: "Twin", name: "twin search", args: "<substring>", summary: "find twinned entities by name" },
    Cmd { group: "Twin", name: "twin insights", args: "<prefix> [--json]", summary: "the synthesized picture: churn, hubs, tests, decisions, growth, last sleep" },
    Cmd { group: "Twin", name: "twin tests", args: "<prefix> [--json]", summary: "test files, frameworks, covers-relations, failing cases" },
    Cmd { group: "Twin", name: "twin stale", args: "<prefix> [--json]", summary: "docs invalidated by later changes to files they mention" },
    Cmd { group: "Twin", name: "ingest", args: "<dir> [--prefix <p>]", summary: "alias for twin refresh" },
    Cmd { group: "Twin", name: "note", args: "<name> <text...> [--kind learning|dead-end|gap|decision-pending]", summary: "attach a durable note to any entity" },
    Cmd { group: "Twin", name: "notes", args: "<name> [--top N] [--json]", summary: "read an entity's notes, in true order" },
    Cmd { group: "Twin", name: "observations", args: "<name>", summary: "an entity's full observation timeline" },
    Cmd { group: "Twin", name: "twin config", args: "<prefix> [--add-extensions csv]", summary: "teach extra file extensions to ingest (additive, size-capped); no flag shows current" },
    Cmd { group: "Twin", name: "relation retract", args: "<from> <predicate> <to> [--prefix <p>]", summary: "mark an edge as no longer holding; history stays, readers stop seeing it" },
    Cmd { group: "Twin", name: "relation list", args: "<name> [--all] [--prefix <p>]", summary: "an entity's live relations, both directions; --all includes retracted" },

    Cmd { group: "Documents", name: "adr add", args: "<md-file> --prefix <p> [--title T] [--status S]", summary: "record a decision from any markdown file" },
    Cmd { group: "Documents", name: "adr list|show", args: "<prefix> [slug]", summary: "decisions with status; show prints text, status timeline, mentions" },
    Cmd { group: "Documents", name: "plan add|list|show", args: "...", summary: "the same for plans (e.g. Claude Code plan files); list hides non-active without --all" },
    Cmd { group: "Documents", name: "plan done|abandon|reopen", args: "<prefix> <slug> [--why R]", summary: "conclude or revive a plan; finished plans stop rotting and leave the lists" },
    Cmd { group: "Documents", name: "artifact set-lifecycle", args: "<prefix> <kind> <slug> <state> [--why R]", summary: "explicitly set any artifact's lifecycle (active|done|abandoned|retired|superseded)" },
    Cmd { group: "Documents", name: "adr ack", args: "<prefix> <slug> [--note T]", summary: "reviewed against current code, still accurate — resets the staleness clock, file untouched" },
    Cmd { group: "Documents", name: "plan ack", args: "<prefix> <slug> [--note T]", summary: "the same acknowledgement for plans" },
    Cmd { group: "Documents", name: "artifact ack", args: "<prefix> <kind> <slug> [--note T]", summary: "the same acknowledgement for any artifact kind" },
    Cmd { group: "Documents", name: "skill add|list|show", args: "...", summary: "agent skills (SKILL.md), auto-captured or added explicitly" },
    Cmd { group: "Documents", name: "agentcfg add|list|show", args: "...", summary: "agent configuration: CLAUDE.md, AGENTS.md, .cursorrules, settings, MCP" },
    Cmd { group: "Documents", name: "template seed|list|show", args: "[slug]", summary: "the deliverable contract as graph data" },
    Cmd { group: "Documents", name: "template set", args: "<slug> --applies-to k --capture <globs> [--fields spec] [--requires a,b] [--rot none|info|warn] [--placement P] [--enforce E] [--home g] [--project-to p] [--extensions e]", summary: "teach the store a new artifact kind — capture, placement, enforcement, and rot policy as data, no code" },
    Cmd { group: "Documents", name: "template fitness", args: "[slug] [--prefix <p>]", summary: "how each contract version performs: first-capture conformance, missed fields, artifact outcomes — the learning loop's read side" },
    Cmd { group: "Documents", name: "template evolve", args: "<slug> [--prefix <p>] [--apply]", summary: "propose the next contract version from fitness evidence; --apply accepts it (never automatic)" },
    Cmd { group: "Documents", name: "artifact new|edit", args: "<prefix> <kind> <slug> [--title T] [--file f|-]", summary: "graph-first authoring: validated at write time (enforced kinds refuse with exit 3), graph_first kinds render a read-only projection" },
    Cmd { group: "Documents", name: "artifact render", args: "[dir] [--prefix <p>] [--kind k] [--check]", summary: "(re)render projections under docs/brain/; --check reports drift, exit 3 on hand-edits" },
    Cmd { group: "Documents", name: "artifact list|show", args: "<prefix> <kind> [slug]", summary: "browse artifacts of any kind, built-in or taught" },
    Cmd { group: "Documents", name: "asset add", args: "<file> --prefix <p> --for <kind>/<slug> [--depicts <t>]... [--subtype s]", summary: "type a binary artifact: owner + declared depicts links give screenshots a staleness story" },
    Cmd { group: "Documents", name: "asset list", args: "<prefix> [--all]", summary: "assets with subtype, path, owner, lifecycle" },
    Cmd { group: "Documents", name: "deliverable new", args: "<template> [--title T]", summary: "instantiate a scaffold from the graph to stdout" },

    Cmd { group: "Tests", name: "testrun import", args: "<report|-> --prefix <p> [--dir <d>]", summary: "ingest cargo-test output, JUnit XML, or Playwright JSON as a content-addressed protocol; Playwright's screenshots, videos and traces become assets owned by the case that produced them" },
    Cmd { group: "Tests", name: "testrun list", args: "<prefix>", summary: "imported protocols, newest first" },

    Cmd { group: "Sessions", name: "sessions import", args: "[dir] [--prefix p] [--agent claude|codex] [--since 2h]", summary: "record the coding-agent sessions that ran in this workspace: objective, model, turns, tools, and the files they edited (never the conversation)" },
    Cmd { group: "Sessions", name: "sessions list", args: "<prefix> [--json]", summary: "who worked here and what they were trying to do, most recent first" },
    Cmd { group: "Sessions", name: "sessions annotate", args: "<prefix> <session-id> [--objective <text>] [--outcome shipped|abandoned|superseded]", summary: "the distilled mission and whether its work survived — supersedes the import-time guess, keeps it in history" },

    Cmd { group: "Features", name: "feature add", args: "<prefix> <slug> [--title T] [--status S] [--part-of <parent>]", summary: "register (or update) a feature; --part-of makes it a part of another feature in one act" },
    Cmd { group: "Features", name: "feature link", args: "<prefix> <slug> <predicate> <target> [--kind k]", summary: "link a feature to files, tests, decisions, docs; part_of joins it to a parent feature" },
    Cmd { group: "Features", name: "feature tree", args: "<prefix> [slug] [--json]", summary: "features and their parts, with readiness rolled up from the leaves" },
    Cmd { group: "Features", name: "feature list|matrix", args: "<prefix>", summary: "the registry; matrix renders the definition of done as a query" },
    Cmd { group: "Features", name: "done", args: "<prefix> <slug>", summary: "evaluate a feature against the DoD and record the outcome" },

    Cmd { group: "Brain functions", name: "wake", args: "<prefix> [--full] [--json]", summary: "orientation: last sleep, the delta since, attention, warn-stale, in-flight work, coherence — one command" },
    Cmd { group: "Brain functions", name: "tidy", args: "[dir] [--prefix <p>] [--fix --cap fs] [--rm <path>]", summary: "clean up: drifted/orphaned projections, retired artifacts, legacy assets, concluded prototypes — fixes are governed changes (auditable, revertible); deletion only via explicit --rm" },
    Cmd { group: "Brain functions", name: "attend", args: "<prefix> [--top N] [--json]", summary: "attention: what deserves attention now, ranked with reasons" },
    Cmd { group: "Brain functions", name: "spine", args: "<prefix> [--unclaimed <kind>] [--json]", summary: "what each feature reaches through the files it declares, what no feature claims, and which declared slots nothing corroborates" },
    Cmd { group: "Brain functions", name: "sleep", args: "<prefix>", summary: "consolidation: distill activity since last sleep into durable memory" },
    Cmd { group: "Brain functions", name: "related", args: "<name> [--top N] [--json]", summary: "association: what is related (co-change, co-mention, shared imports)" },
    Cmd { group: "Brain functions", name: "find", args: "<prefix> <query...> [--top N] [--json]", summary: "where is the thing that does X — paths, symbol names, doc titles, notes, ranked by graph centrality" },
    Cmd { group: "Brain functions", name: "next", args: "<prefix> [--top N] [--json]", summary: "the ranked work queue: failing tests, unsettled changes, rotting docs, unmet definitions of done, open plans" },
    Cmd { group: "Brain functions", name: "before", args: "<name> [--json]", summary: "the pre-edit briefing: write access, blast radius, covering tests, constraining docs, churn, notes, associations" },
    Cmd { group: "Brain functions", name: "can-i", args: "<name-or-path> [--prefix <p>] [--json]", summary: "the authoring gate as a question — exit 0: write the file; exit 3: the graph owns it, the answer names the fix" },
    Cmd { group: "Brain functions", name: "eyes", args: "[--prefix P] [--bind IP] [--port N] [--root DIR]", summary: "the visual layer for people: judgments as sentences, content you can read, a map of the system \u{2014} read-only, on localhost" },

    Cmd { group: "Governed changes", name: "change propose", args: "<prefix> <path> --from <file> [--reason R]", summary: "propose a change: pure graph write, disk untouched" },
    Cmd { group: "Governed changes", name: "change apply|revert", args: "<prefix> <slug> --cap fs", summary: "through intent -> write -> receipt; refused without the capability" },
    Cmd { group: "Governed changes", name: "change verify", args: "<prefix> <slug>", summary: "run the repo's test command, link the protocol, grade the change" },
    Cmd { group: "Governed changes", name: "change list|show", args: "<prefix> [slug]", summary: "the change ledger with status timelines" },
    Cmd { group: "Governed changes", name: "recover", args: "", summary: "mark pending intents indeterminate and reconcile changes — never retries" },

    Cmd { group: "Automation", name: "hook install", args: "[dir] [--prefix p] [--docs] [--tests] [--test-cmd c] [--gate]", summary: "every git commit/push/checkout/merge triggers the brain; --tests runs the suite, --gate adds the opt-in pre-commit refusal (exit 3 blocks; errors fail open)" },
    Cmd { group: "Automation", name: "hook status|uninstall", args: "[dir]", summary: "inspect or remove the hooks (foreign hooks respected)" },
    Cmd { group: "Automation", name: "watch", args: "[dir] [--prefix p] [--interval s] [--docs]", summary: "continuous refresh + insights loop, built into the binary" },
    Cmd { group: "Automation", name: "instructions generate", args: "[dir] [--prefix p] [--check]", summary: "render one guardrail block from the kind registry into CLAUDE.md and AGENTS.md — every agent family reads identical rules" },
    Cmd { group: "Automation", name: "docs generate", args: "[dir] [--prefix p] [--out d]", summary: "regenerate docs/generated/: tour, screenshots, narrated screencast, man page" },

    Cmd { group: "Native code", name: "put-code", args: "<name> <term>", summary: "store a term (.json or .term notation) and bind it" },
    Cmd { group: "Native code", name: "notation", args: "<file>", summary: "convert a term between compact notation and JSON" },
    Cmd { group: "Native code", name: "run", args: "<name> [--cap <c>]...", summary: "evaluate bound code; effects require capabilities" },
    Cmd { group: "Native code", name: "task check", args: "<task.json> <term>", summary: "check a solution, record evidence (cached across alpha-equivalents)" },
    Cmd { group: "Native code", name: "demo", args: "", summary: "the end-to-end demonstration" },

    Cmd { group: "Utilities", name: "refs", args: "<name|b3:hash>", summary: "reverse edges: who references this node" },
    Cmd { group: "Utilities", name: "deps", args: "<name|b3:hash>", summary: "forward edges: what this node references" },
    Cmd { group: "Utilities", name: "evidence", args: "<name|b3:hash>", summary: "verification claims about a node" },
    Cmd { group: "Utilities", name: "bench index", args: "[--prefix <p>]", summary: "cortex vs cold replay — answers verified identical before timing" },
    Cmd { group: "Utilities", name: "man", args: "[--install] [--out <file>]", summary: "this manual (troff); --install writes to ~/.local/share/man" },
    Cmd { group: "Utilities", name: "version", args: "", summary: "print the version" },
];

pub const ENVIRONMENT: &[(&str, &str)] = &[
    ("BRAIN_STORE", "store directory (default ./.brain)"),
    ("BRAIN_INDEX", "set to 'mem' to force a cold reference-backend rebuild instead of the cortex checkpoint"),
    ("BRAIN_TTS_MODEL", "TTS model for docs narration (default Qwen/Qwen3-TTS-12Hz-0.6B-Base; espeak-ng is the fallback)"),
];

pub const FILES: &[(&str, &str)] = &[
    (
        ".brain/objects/",
        "immutable content-addressed objects (canonical JSON; identity = BLAKE3 of bytes)",
    ),
    (
        ".brain/events.jsonl",
        "append-only event log — the WAL every derived index replays",
    ),
    (
        ".brain/intents.jsonl",
        "durable intent/receipt state for the effect boundary",
    ),
    (".brain/HEAD", "NodeId of the current namespace object"),
    (
        ".brain/cortex.json",
        "the cortex checkpoint — derived, disposable, rebuilt if missing",
    ),
];

/// The `--help`/usage projection.
pub fn usage_text() -> String {
    let mut out = String::from(
        "brain — agent-native semantic substrate: one graph for code, twin, decisions, tests, and docs\n\nUsage: brain <command> [args]\n",
    );
    for (group, _) in GROUPS {
        let _ = writeln!(out, "\n  {group}:");
        for c in COMMANDS.iter().filter(|c| c.group == *group) {
            let left = if c.args.is_empty() {
                format!("brain {}", c.name)
            } else {
                format!("brain {} {}", c.name, c.args)
            };
            let _ = writeln!(out, "    {left:<58} {}", c.summary);
        }
    }
    out.push_str(
        "\nFull manual: brain man | man -l -    (or: brain man --install; then: man brain)\n",
    );
    out
}

fn troff_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('—', "\\(em")
        .replace('-', "\\-")
}

/// The man(1) projection.
pub fn man_page() -> String {
    let mut o = String::new();
    let _ = writeln!(
        o,
        ".TH BRAIN 1 \"\" \"brain {}\" \"User Commands\"",
        env!("CARGO_PKG_VERSION")
    );
    o.push_str(".SH NAME\nbrain \\- agent\\-native semantic substrate: one graph for code, twin, decisions, tests, and docs\n");
    o.push_str(".SH SYNOPSIS\n.B brain\n.I command\n.RI [ args ]\n");
    o.push_str(".SH DESCRIPTION\n");
    o.push_str(
        "brain keeps software knowledge in a content\\-addressed graph: what the software is (files, symbols, imports), why it is that way (decisions, plans), how it is built (skills, agent configuration), what done means (templates, a definition of done), what happened (test protocols, timelines), and what matters now (attention). Facts are immutable, sourced, timestamped observations \\- history is the data model, not a feature. Git hooks keep the graph fresh on every commit; replication moves truth between stores as a set union.\n",
    );
    for (group, desc) in GROUPS {
        let _ = writeln!(o, ".SH {}", troff_escape(&group.to_uppercase()));
        let _ = writeln!(o, "{}", troff_escape(desc));
        for c in COMMANDS.iter().filter(|c| c.group == *group) {
            o.push_str(".TP\n");
            if c.args.is_empty() {
                let _ = writeln!(o, ".B brain {}", troff_escape(c.name));
            } else {
                let _ = writeln!(
                    o,
                    ".B brain {} \\fI{}\\fR",
                    troff_escape(c.name),
                    troff_escape(c.args)
                );
            }
            let _ = writeln!(o, "{}", troff_escape(c.summary));
        }
    }
    o.push_str(".SH ENVIRONMENT\n");
    for (var, desc) in ENVIRONMENT {
        let _ = writeln!(o, ".TP\n.B {var}\n{}", troff_escape(desc));
    }
    o.push_str(".SH FILES\n");
    for (file, desc) in FILES {
        let _ = writeln!(o, ".TP\n.B {}\n{}", troff_escape(file), troff_escape(desc));
    }
    o.push_str(".SH EXAMPLES\n");
    o.push_str(".SS Brownfield minute\\-one\n.nf\nbrain init\nbrain twin backfill . \\-\\-prefix twin/app\nbrain twin refresh  . \\-\\-prefix twin/app\nbrain hook install \\-\\-tests\nbrain attend twin/app\n.fi\n");
    o.push_str(".SS The session rhythm\n.nf\nbrain attend twin/app          # wake: what matters now\n...work; commits refresh the twin and run the suite...\nbrain twin stale twin/app      # fix rotted docs\nbrain sleep twin/app           # consolidate before you go\n.fi\n");
    o.push_str(".SH EXIT STATUS\n0 on success, 1 on error, 2 on usage error.\n");
    o.push_str(".SH SEE ALSO\ndocs/twin.md, docs/architecture.md, docs/calculus.md, and the ADRs under docs/adr/ \\- all captured in the graph itself (\\fBbrain adr list twin/self\\fR).\n");
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_coherent_and_both_projections_render() {
        // No duplicate command names; every command has a group and prose.
        let mut names: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
        names.sort();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "duplicate command names");
        for c in COMMANDS {
            assert!(
                GROUPS.iter().any(|(g, _)| g == &c.group),
                "unknown group {}",
                c.group
            );
            assert!(!c.summary.is_empty());
        }

        // Usage lists every command; man page is structurally troff.
        let usage = usage_text();
        for c in COMMANDS {
            assert!(usage.contains(c.name), "usage missing {}", c.name);
        }
        let man = man_page();
        assert!(man.starts_with(".TH BRAIN 1"));
        for (group, _) in GROUPS {
            assert!(man.contains(&format!(".SH {}", group.to_uppercase().replace('-', "\\-"))));
        }
        assert!(man.contains(".B brain twin\\-refresh") || man.contains(".B brain twin refresh"));
        assert!(man.contains("BRAIN_STORE"));
        assert!(man.contains("cortex.json"));
    }
}
