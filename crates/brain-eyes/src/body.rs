//! Reading an artifact's actual bytes.
//!
//! Text comes from the entity's latest `content` observation — the graph
//! snapshot, not the working tree. Binary bodies stay file-first (ADR-018)
//! and are read only through a graph-recorded relative path that resolves
//! inside the configured workspace root. The browser never supplies a
//! path, and Eyes claims verification **only** when a recorded
//! `content_b3` was actually there to check against.

use crate::dto::Body;
use crate::state::Loaded;
use brain_core::ids::StableId;
use brain_observe::twin;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const MAX_INLINE_BODY_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_RAW_BODY_BYTES: usize = 64 * 1024 * 1024;

pub struct Resolved {
    pub view: Body,
    pub bytes: Vec<u8>,
}

/// Reject anything that is not a plain relative path inside the root.
/// Canonicalising *after* joining also catches an in-workspace symlink
/// that points outside.
pub fn safe_content_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("that is not a safe workspace path".to_string());
    }
    let root = fs::canonicalize(root)
        .map_err(|error| format!("the workspace root is unavailable: {error}"))?;
    let path = fs::canonicalize(root.join(relative))
        .map_err(|error| format!("this file is not in the workspace: {error}"))?;
    if !path.starts_with(&root) {
        return Err("that path leaves the workspace".to_string());
    }
    Ok(path)
}

pub fn resolve(
    loaded: &Loaded,
    sid: &StableId,
    kind: &str,
    labels: &BTreeMap<String, String>,
    content_root: Option<&Path>,
) -> Result<Resolved, String> {
    let store = &loaded.store;
    let index = &loaded.index;
    let path = labels.get("path").cloned();

    let (bytes, origin, verified) = match twin::latest(index, store, sid, "content")
        .map_err(|e| e.to_string())?
    {
        Some(content) => (
            content.into_bytes(),
            "recorded in the graph".to_string(),
            true,
        ),
        None => {
            let relative = path
                .as_deref()
                .ok_or_else(|| "nothing readable is recorded for this".to_string())?;
            let root = content_root
                .ok_or_else(|| "workspace files are not being served".to_string())?;
            let file = safe_content_path(root, relative)?;
            let metadata = fs::metadata(&file)
                .map_err(|error| format!("this file is unavailable: {error}"))?;
            if metadata.len() > MAX_RAW_BODY_BYTES as u64 {
                return Err(too_large());
            }
            let bytes =
                fs::read(&file).map_err(|error| format!("this file is unavailable: {error}"))?;

            // Verify against the hash the graph recorded — and say so
            // honestly when there is nothing to verify against.
            let file_sid = StableId::derive(&["file", relative]);
            match twin::latest(index, store, &file_sid, "content_b3")
                .map_err(|e| e.to_string())?
            {
                Some(expected) => {
                    let actual = brain_core::canonical::hash_bytes(&bytes).to_string();
                    let actual = actual.strip_prefix("b3:").unwrap_or(&actual);
                    if actual != expected {
                        return Err(
                            "this file changed after the graph last looked at it — refresh the twin"
                                .to_string(),
                        );
                    }
                    (
                        bytes,
                        "from the workspace, matching what the graph recorded".to_string(),
                        true,
                    )
                }
                None => (
                    bytes,
                    "from the workspace — the graph has no record to check it against"
                        .to_string(),
                    false,
                ),
            }
        }
    };

    if bytes.len() > MAX_RAW_BODY_BYTES {
        return Err(too_large());
    }
    let (format, media_type) = body_format(path.as_deref(), kind, &bytes);
    let textual = matches!(format.as_str(), "markdown" | "json" | "code" | "text");
    let (text, truncated) = if textual {
        let value = String::from_utf8(bytes.clone())
            .map_err(|_| "this looks like text but is not readable as text".to_string())?;
        if value.len() > MAX_INLINE_BODY_BYTES {
            let mut end = MAX_INLINE_BODY_BYTES;
            while !value.is_char_boundary(end) {
                end -= 1;
            }
            (Some(value[..end].to_string()), true)
        } else {
            (Some(value), false)
        }
    } else {
        (None, false)
    };

    Ok(Resolved {
        view: Body {
            format,
            media_type,
            origin,
            verified,
            path,
            size_bytes: bytes.len(),
            text,
            truncated,
        },
        bytes,
    })
}

fn too_large() -> String {
    format!(
        "this file is bigger than the {} MB Eyes will read",
        MAX_RAW_BODY_BYTES / 1024 / 1024
    )
}

pub fn body_format(path: Option<&str>, kind: &str, bytes: &[u8]) -> (String, String) {
    let extension = path
        .and_then(|value| Path::new(value).extension())
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let pair = match extension.as_str() {
        "md" | "mdx" => ("markdown", "text/markdown"),
        "json" => ("json", "application/json"),
        "txt" | "log" | "csv" | "tsv" => ("text", "text/plain"),
        "rs" | "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" | "css" | "scss" | "html" | "htm"
        | "toml" | "yaml" | "yml" | "xml" | "svg" | "sh" | "zsh" | "bash" | "py" | "php"
        | "go" | "java" | "kt" | "swift" | "c" | "h" | "cpp" | "hpp" | "sql" | "jsonl"
        | "mdc" | "1" => ("code", "text/plain"),
        "png" => ("image", "image/png"),
        "jpg" | "jpeg" => ("image", "image/jpeg"),
        "gif" => ("image", "image/gif"),
        "webp" => ("image", "image/webp"),
        "pdf" => ("pdf", "application/pdf"),
        "mp3" => ("audio", "audio/mpeg"),
        "wav" => ("audio", "audio/wav"),
        "ogg" => ("audio", "audio/ogg"),
        "m4a" => ("audio", "audio/mp4"),
        "mp4" => ("video", "video/mp4"),
        "webm" => ("video", "video/webm"),
        _ if [
            "decision",
            "doc",
            "plan",
            "runbook",
            "task_list",
            "capability_matrix",
            "skill",
            "agent_config",
            "template",
            "prototype",
        ]
        .contains(&kind) =>
        {
            ("markdown", "text/markdown")
        }
        _ if std::str::from_utf8(bytes).is_ok() => ("text", "text/plain"),
        _ => ("binary", "application/octet-stream"),
    };
    (pair.0.to_string(), pair.1.to_string())
}
