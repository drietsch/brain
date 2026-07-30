//! brain-observe: reflective mode — the graph as a twin of external software.
//!
//! The twin models software that lives elsewhere as entities, time-bound
//! observations, and typed relations, sharing one identity scheme with
//! native-mode code so a twinned thing can later gain a native
//! implementation without re-modeling.
//!
//! - [`twin`] — the engine: drift-aware `refresh`, read-only `status`,
//!   agent notes.
//! - [`symbols`] — lightweight per-language symbol and import extraction.
//! - [`docs`] — parsing for decision records (ADRs) and plans, the *why*
//!   documents captured alongside structure.
//! - [`agents`] — parsing for skills and agent configuration (CLAUDE.md,
//!   AGENTS.md, .cursorrules, subagents, settings): *how it is built*.
//! - [`templates`] — the deliverable contract as graph data: scaffolds,
//!   required fields, and recorded (never enforced) conformance.
//! - [`features`] — the feature registry: done-ness as a graph query
//!   against the template-defined definition of done.
//! - [`testing`] — tests as graph citizens: framework classification,
//!   `covers` relations, and test-run protocols with result timelines.
//!
//! Observers are sense organs, meant to run continuously; re-ingesting
//! refreshes observations and surfaces drift as new nodes, never overwrites.

pub mod agenda;
pub mod agents;
pub mod assets;
pub mod assoc;
pub mod attention;
pub mod backfill;
pub mod briefing;
pub mod coherence;
pub mod docs;
pub mod features;
pub mod find;
pub mod fitness;
pub mod govern;
pub mod instructions;
pub mod kinds;
pub mod lifecycle;
pub mod projection;
pub mod sessions;
pub mod sleep;
pub mod spine;
pub mod symbols;
pub mod templates;
pub mod testing;
pub mod tidy;
pub mod tour;
pub mod twin;
pub mod wake;

use std::fs;
use std::path::Path;

use brain_store::StoreError;

/// File extensions worth twinning in a source tree. Media formats are
/// included so generated documentation artifacts (screenshots, screencasts,
/// narration) carry freshness observations like any other file; `txt` and
/// `1` so the docs pipeline's own outputs (narration.txt, brain.1) are
/// projections the graph can verify.
pub(crate) const INGEST_EXTENSIONS: &[&str] = &[
    "rs", "toml", "md", "json", "php", "py", "js", "jsx", "ts", "tsx", "mdc", "sh", "mjs", "png",
    "svg", "gif", "webm", "mp4", "wav", "txt", "1",
];
/// Extensionless files worth twinning by exact name (agent configuration).
pub(crate) const INGEST_FILENAMES: &[&str] = &[".cursorrules"];
/// Directories that are build products or substrate internals, not software.
pub(crate) const SKIP_DIRS: &[&str] = &[".git", "target", ".brain", "node_modules", "vendor"];

/// Extra-extension files larger than this stay invisible — rule-driven
/// ingestion is for artifacts, not archives.
pub(crate) const MAX_EXTRA_FILE: u64 = 1024 * 1024;

/// Runtime-taught ingestion beyond the compiled extension list. Two
/// layers, both additive, both size-capped: repo-level extensions (an
/// explicit `ingest_extensions` observation) apply everywhere; a kind's
/// `extensions` apply only where its capture/home globs reach — teaching
/// `extensions=jsonl` on a kind capturing `runs/*.jsonl` ingests exactly
/// those files, and a stray archive elsewhere stays invisible.
#[derive(Debug, Default)]
pub(crate) struct ExtraIngest {
    pub repo_exts: std::collections::BTreeSet<String>,
    /// (extensions, globs) per registry kind that teaches extensions.
    pub kind_rules: Vec<(std::collections::BTreeSet<String>, Vec<String>)>,
}

impl ExtraIngest {
    pub(crate) fn is_empty(&self) -> bool {
        self.repo_exts.is_empty() && self.kind_rules.is_empty()
    }

    fn keep(&self, rel: &str, ext: &str) -> bool {
        self.repo_exts.contains(ext)
            || self.kind_rules.iter().any(|(exts, globs)| {
                exts.contains(ext) && globs.iter().any(|g| templates::glob_match(g, rel))
            })
    }
}

pub(crate) fn collect_files_with(
    root: &Path,
    dir: &Path,
    extra: &ExtraIngest,
    out: &mut Vec<String>,
) -> Result<(), StoreError> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.file_type()?.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            collect_files_with(root, &path, extra, out)?;
        } else {
            let ext = path.extension().and_then(|e| e.to_str());
            let rel = match path.strip_prefix(root) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let builtin = ext.is_some_and(|e| INGEST_EXTENSIONS.contains(&e))
                || INGEST_FILENAMES.contains(&name.as_str());
            let taught = !builtin
                && !extra.is_empty()
                && ext.is_some_and(|e| extra.keep(&rel, e))
                && entry
                    .metadata()
                    .map(|m| m.len() <= MAX_EXTRA_FILE)
                    .unwrap_or(false);
            if builtin || taught {
                out.push(rel);
            }
        }
    }
    Ok(())
}
