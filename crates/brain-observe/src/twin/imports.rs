//! Resolving one file's mention of another into a path the graph can hold.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

/// Best-effort resolution of an import string to a twinned file path.
pub(crate) fn resolve_import(from_rel: &str, import: &str, files: &BTreeSet<String>) -> Option<String> {
    if files.contains(import) {
        return Some(import.to_string());
    }
    // Rust intra-crate: `crate::foo::Bar` -> <src-root>/foo.rs or foo/mod.rs,
    // where the src root is the importing file's path up through "src/".
    // Item imports (`crate::helper`) fall back to the crate root (lib.rs).
    if let Some(rest) = import.strip_prefix("crate::") {
        let src_root = if let Some(p) = from_rel.rfind("/src/") {
            &from_rel[..p + 5]
        } else if from_rel.starts_with("src/") {
            "src/"
        } else {
            ""
        };
        if let Some(first) = rest.split("::").next() {
            for cand in [
                format!("{src_root}{first}.rs"),
                format!("{src_root}{first}/mod.rs"),
                format!("{src_root}lib.rs"),
            ] {
                if files.contains(&cand) {
                    return Some(cand);
                }
            }
        }
    }
    // Rust cross-crate: `foo_bar::mod::Item` resolves into a sibling
    // crate's src tree when one exists among the walked files (crate dirs
    // may use hyphens where imports use underscores). Item and bare-crate
    // imports fall back to the crate root — the honest answer for
    // `use foo_bar::Thing` and `use foo_bar::{a, b}`.
    if !import.contains('/') {
        let mut segs = import.split("::");
        let first = segs.next().unwrap_or("");
        let second = segs.next();
        if !matches!(first, "crate" | "super" | "self" | "std" | "core" | "alloc")
            && first.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !first.is_empty()
        {
            let hyphen = first.replace('_', "-");
            for f in files {
                let Some(src_root) = f.strip_suffix("lib.rs") else {
                    continue;
                };
                let dir = src_root
                    .strip_suffix("/src/")
                    .map(|d| d.rsplit('/').next().unwrap_or(d))
                    .unwrap_or("");
                if dir.is_empty() || (dir != first && dir != hyphen) {
                    continue;
                }
                if let Some(second) = second {
                    for cand in [
                        format!("{src_root}{second}.rs"),
                        format!("{src_root}{second}/mod.rs"),
                    ] {
                        if files.contains(&cand) {
                            return Some(cand);
                        }
                    }
                }
                return Some(f.clone());
            }
        }
    }
    if import.starts_with("./") || import.starts_with("../") {
        let dir = match from_rel.rsplit_once('/') {
            Some((d, _)) => d,
            None => "",
        };
        let joined = normalize(&if dir.is_empty() {
            import.to_string()
        } else {
            format!("{dir}/{import}")
        });
        for suffix in ["", ".js", ".ts", ".jsx", ".tsx", ".py", ".php", ".rs"] {
            let cand = format!("{joined}{suffix}");
            if files.contains(&cand) {
                return Some(cand);
            }
        }
        for idx in ["/index.js", "/index.ts"] {
            let cand = format!("{joined}{idx}");
            if files.contains(&cand) {
                return Some(cand);
            }
        }
    }
    None
}

/// Collapse `.` and `..` components in a relative path.
pub(crate) fn normalize(p: &str) -> String {
    let mut stack: Vec<&str> = Vec::new();
    for part in p.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            other => stack.push(other),
        }
    }
    stack.join("/")
}

pub(crate) fn git_info(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (prop, args) in [
        ("git_commit", ["rev-parse", "HEAD"]),
        ("git_branch", ["rev-parse", "--abbrev-ref"]),
    ] {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(root).args(args);
        if prop == "git_branch" {
            cmd.arg("HEAD");
        }
        if let Ok(o) = cmd.output() {
            if o.status.success() {
                let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if !v.is_empty() {
                    out.push((prop.to_string(), v));
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Notes: durable agent memory attached to any entity
// ---------------------------------------------------------------------------
