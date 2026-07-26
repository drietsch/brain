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
//!
//! Observers are sense organs, meant to run continuously; re-ingesting
//! refreshes observations and surfaces drift as new nodes, never overwrites.

pub mod symbols;
pub mod twin;

use std::fs;
use std::path::Path;

use brain_store::StoreError;

/// File extensions worth twinning in a source tree.
pub(crate) const INGEST_EXTENSIONS: &[&str] = &[
    "rs", "toml", "md", "json", "php", "py", "js", "jsx", "ts", "tsx",
];
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
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if INGEST_EXTENSIONS.contains(&ext) {
                if let Ok(rel) = path.strip_prefix(root) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    Ok(())
}
