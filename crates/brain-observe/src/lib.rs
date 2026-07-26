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

pub mod agents;
pub mod assoc;
pub mod attention;
pub mod docs;
pub mod features;
pub mod sleep;
pub mod symbols;
pub mod templates;
pub mod testing;
pub mod twin;

use std::fs;
use std::path::Path;

use brain_store::StoreError;

/// File extensions worth twinning in a source tree. Media formats are
/// included so generated documentation artifacts (screenshots, screencasts,
/// narration) carry freshness observations like any other file.
pub(crate) const INGEST_EXTENSIONS: &[&str] = &[
    "rs", "toml", "md", "json", "php", "py", "js", "jsx", "ts", "tsx", "mdc", "sh", "mjs",
    "png", "svg", "gif", "webm", "mp4", "wav",
];
/// Extensionless files worth twinning by exact name (agent configuration).
pub(crate) const INGEST_FILENAMES: &[&str] = &[".cursorrules"];
/// Directories that are build products or substrate internals, not software.
pub(crate) const SKIP_DIRS: &[&str] = &[".git", "target", ".brain", "node_modules", "vendor"];

pub(crate) fn collect_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), StoreError> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.file_type()?.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            collect_files(root, &path, out)?;
        } else {
            let ext = path.extension().and_then(|e| e.to_str());
            let keep = ext.is_some_and(|e| INGEST_EXTENSIONS.contains(&e))
                || INGEST_FILENAMES.contains(&name.as_str());
            if keep {
                if let Ok(rel) = path.strip_prefix(root) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    Ok(())
}
