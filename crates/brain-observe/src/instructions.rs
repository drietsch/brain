//! Instruction-file projection: one source of guardrails for every agent.
//!
//! Claude Code reads CLAUDE.md, Codex reads AGENTS.md, and prose
//! conventions drift apart the moment two files exist. This module
//! renders ONE deterministic guardrail block from the kind registry and
//! maintains it inside managed markers in both files — content outside
//! the markers is never touched, and the block's hash is recorded on the
//! file entity (`instructions_b3`) so an in-block hand edit is detectable
//! drift, not silent divergence.

use crate::kinds;
use crate::twin::{latest, observe_src};
use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::MemIndex;
use brain_store::{now_ms, Store, StoreError};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub const BEGIN: &str = "<!-- brain:begin instructions — generated from the kind registry by `brain instructions generate`; edit rules with `brain template set`, never here -->";
pub const END: &str = "<!-- brain:end instructions -->";

/// The instruction files every agent family reads. Identical content.
pub const TARGETS: &[&str] = &["CLAUDE.md", "AGENTS.md"];

/// Render the guardrail block from the registry. Deterministic: sorted
/// kinds, no timestamps.
pub fn block(store: &Store, index: &MemIndex, prefix: &str) -> Result<String, StoreError> {
    let registry = kinds::registry(store, index)?;
    let mut out = String::new();
    out.push_str(BEGIN);
    out.push_str("\n\n## Brain guardrails\n\n");
    out.push_str(&format!(
        "Orient with `brain wake {prefix}` before working; consolidate with `brain sleep {prefix}` before finishing.\n\n"
    ));
    out.push_str("Artifact kinds (where truth lives, how to author):\n\n");
    out.push_str("| kind | placement | lives at | author via | requires | enforcement |\n");
    out.push_str("|---|---|---|---|---|---|\n");
    for (kind, def) in &registry {
        // A kind with no file anywhere still has to appear. A taught,
        // graph-only kind that is missing from this table is a kind no
        // agent knows exists — which is the opposite of what teaching one
        // is for.
        let home = if !def.project_to.is_empty() {
            def.project_to.clone()
        } else if !def.home.is_empty() {
            def.home.join(", ")
        } else if !def.capture.is_empty() {
            def.capture.join(", ")
        } else {
            "in the graph only".to_string()
        };
        let author = match def.placement.as_str() {
            "graph_first" => format!("`brain artifact new {prefix} {kind} <slug>`"),
            "projection" => "rendered query — never authored".to_string(),
            _ => "write the file; the twin captures it".to_string(),
        };
        out.push_str(&format!(
            "| {kind} | {} | {home} | {author} | {} | {} |\n",
            def.placement,
            def.requires.join(", "),
            def.enforce
        ));
    }
    out.push_str(&format!(
        "\nRules:\n\n\
         - Files under `docs/brain/` are **read-only projections** of the graph. Edit through `brain artifact edit {prefix} <kind> <slug>`, never the file.\n\
         - Finished plans: `brain plan done {prefix} <slug>`. A doc reviewed and still accurate: `brain adr|plan|artifact ack` (resets its staleness clock).\n\
         - A wrong or outdated link: `brain relation retract <from> <predicate> <to>`.\n\
         - Binary assets (screenshots, HTML templates): `brain asset add <file> --prefix {prefix} --for <kind>/<slug> --depicts <path>` — declared links are their staleness story.\n\
         - Enforced kinds refuse nonconforming writes (exit 3) and, with the pre-commit gate, block commits; the error names the fix.\n\n"
    ));
    out.push_str(END);
    out.push('\n');
    Ok(out)
}

/// Splice the block into a file's content: replace between markers, or
/// append (with a separating blank line) when absent.
pub fn splice(existing: &str, block: &str) -> String {
    if let (Some(b), Some(e)) = (existing.find(BEGIN), existing.find(END)) {
        if e >= b {
            let mut out = String::new();
            out.push_str(&existing[..b]);
            out.push_str(block.trim_end());
            out.push('\n');
            out.push_str(existing[e + END.len()..].trim_start_matches('\n'));
            return out;
        }
    }
    let mut out = existing.to_string();
    if !out.is_empty() && !out.ends_with("\n\n") {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    out.push_str(block);
    out
}

/// Generate/refresh the managed block in every target file. Returns
/// (file, changed) pairs. The block hash is recorded on each file entity.
pub fn generate(
    store: &Store,
    index: &MemIndex,
    root: &Path,
    prefix: &str,
) -> Result<Vec<(String, bool)>, StoreError> {
    let block = block(store, index, prefix)?;
    let hash = blake3::hash(block.as_bytes()).to_hex().to_string();
    let now = now_ms();
    let mut out = Vec::new();
    for target in TARGETS {
        let path = root.join(target);
        let existing = fs::read_to_string(&path).unwrap_or_default();
        let next = splice(&existing, &block);
        let changed = next != existing;
        if changed {
            fs::write(&path, &next)?;
        }
        let sid = StableId::derive(&["file", target]);
        let mut labels = BTreeMap::new();
        labels.insert("path".to_string(), target.to_string());
        store.put(&Object::Entity {
            id: sid.clone(),
            entity_kind: "source_file".to_string(),
            labels,
        })?;
        if latest(index, store, &sid, "instructions_b3")?.as_deref() != Some(hash.as_str()) {
            observe_src(store, &sid, "instructions_b3", &hash, "projection", now)?;
        }
        out.push((target.to_string(), changed));
    }
    Ok(out)
}

/// Files whose in-file block no longer matches the recorded hash (hand
/// edit inside the markers) or that lack the block entirely while the
/// registry expects one. Consumed by tidy and `--check`.
pub fn block_drift(
    store: &Store,
    index: &MemIndex,
    root: &Path,
    prefix: &str,
) -> Result<Vec<String>, StoreError> {
    let expected = block(store, index, prefix)?;
    let mut out = Vec::new();
    for target in TARGETS {
        let sid = StableId::derive(&["file", target]);
        let Some(_) = latest(index, store, &sid, "instructions_b3")? else {
            continue;
        };
        let existing = fs::read_to_string(root.join(target)).unwrap_or_default();
        let current_block = match (existing.find(BEGIN), existing.find(END)) {
            (Some(b), Some(e)) if e >= b => {
                format!("{}\n", &existing[b..e + END.len()])
            }
            _ => String::new(),
        };
        if current_block != expected {
            out.push(target.to_string());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain_index::replay;

    #[test]
    fn managed_block_splices_without_touching_surrounding_prose() {
        let root = tempfile::tempdir().unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        crate::templates::seed(&store).unwrap();
        fs::write(
            root.path().join("CLAUDE.md"),
            "# My project\n\nHand-written intro.\n",
        )
        .unwrap();

        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        let results = generate(&store, &index, root.path(), "twin/app").unwrap();
        assert!(results.iter().all(|(_, changed)| *changed));

        let claude = fs::read_to_string(root.path().join("CLAUDE.md")).unwrap();
        assert!(
            claude.starts_with("# My project"),
            "prose preserved: {claude}"
        );
        assert!(claude.contains(BEGIN) && claude.contains(END));
        assert!(
            claude.contains("brain artifact new twin/app plan"),
            "{claude}"
        );
        let agents = fs::read_to_string(root.path().join("AGENTS.md")).unwrap();
        // Identical guardrails for every agent family.
        let block_of = |s: &str| {
            let b = s.find(BEGIN).unwrap();
            let e = s.find(END).unwrap();
            s[b..e].to_string()
        };
        assert_eq!(block_of(&claude), block_of(&agents));

        // Idempotent: regenerate changes nothing, writes nothing.
        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        let before = store.count_objects().unwrap();
        let results = generate(&store, &index, root.path(), "twin/app").unwrap();
        assert!(results.iter().all(|(_, changed)| !*changed));
        assert_eq!(store.count_objects().unwrap(), before);
        assert!(block_drift(&store, &index, root.path(), "twin/app")
            .unwrap()
            .is_empty());

        // An in-block hand edit is detected.
        let edited = claude.replace("read-only projections", "editable files");
        fs::write(root.path().join("CLAUDE.md"), edited).unwrap();
        let drifted = block_drift(&store, &index, root.path(), "twin/app").unwrap();
        assert_eq!(drifted, vec!["CLAUDE.md".to_string()]);
    }
}
