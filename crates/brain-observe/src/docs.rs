//! Parsing for decision records (ADRs) and plans — the *why* documents.
//!
//! Pure functions only; recording into the graph lives in [`crate::twin`].
//! Detection is conventional-path based, parsing is line-based and
//! forgiving: a title is the first `# ` heading, an ADR status is a
//! `Status:` line or the line under a `## Status` heading.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    Decision,
    Plan,
}

impl DocKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            DocKind::Decision => "decision",
            DocKind::Plan => "plan",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DocMeta {
    pub kind: DocKind,
    pub slug: String,
    pub title: String,
    /// Decisions default to "recorded"; plans carry a status only when the
    /// document declares one (`Status: done` marks a plan finished).
    pub status: Option<String>,
    /// Slug of a decision this one supersedes, when declared.
    pub supersedes: Option<String>,
}

/// Does this path follow an ADR/plan convention? Path-only, no content read.
pub fn path_kind(rel_path: &str) -> Option<(DocKind, String)> {
    if !rel_path.ends_with(".md") {
        return None;
    }
    let lower = rel_path.to_lowercase();
    let file = lower.rsplit('/').next().unwrap_or(&lower);
    let kind = if lower.contains("/adr/")
        || lower.contains("/decisions/")
        || lower.starts_with("adr/")
        || lower.starts_with("decisions/")
        || file.starts_with("adr-")
    {
        DocKind::Decision
    } else if lower.contains("/plans/") || lower.starts_with("plans/") {
        DocKind::Plan
    } else {
        return None;
    };
    Some((kind, slug_of(rel_path)))
}

/// Filename stem (any extension stripped), lowercased: the document's
/// stable identity within a prefix.
pub fn slug_of(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    let stem = match file.rsplit_once('.') {
        Some((s, _)) if !s.is_empty() => s,
        _ => file,
    };
    stem.to_lowercase()
}

/// Parse a document whose kind was detected from its path.
pub fn parse_doc(rel_path: &str, content: &str) -> Option<DocMeta> {
    let (kind, slug) = path_kind(rel_path)?;
    Some(parse_content(kind, &slug, content, None, None))
}

/// Parse any markdown as a document of a known kind — the explicit-add path
/// for files outside the observed repo (e.g. Claude Code plan files).
pub fn parse_content(
    kind: DocKind,
    slug: &str,
    content: &str,
    title_override: Option<&str>,
    status_override: Option<&str>,
) -> DocMeta {
    let mut title = None;
    let mut status = None;
    let mut supersedes = None;
    let mut lines = content.lines().peekable();
    while let Some(raw) = lines.next() {
        let line = raw.trim();
        if title.is_none() {
            if let Some(rest) = line.strip_prefix("# ") {
                title = Some(rest.trim().to_string());
            }
        }
        if status.is_none() {
            let l = line.to_lowercase();
            if let Some(rest) = line.get(7..).filter(|_| l.starts_with("status:")) {
                let v = rest.trim().to_lowercase();
                if !v.is_empty() {
                    status = Some(v);
                }
            } else if l == "## status" {
                for next in lines.by_ref() {
                    let n = next.trim();
                    if !n.is_empty() {
                        status = Some(n.to_lowercase());
                        break;
                    }
                }
            }
        }
        if supersedes.is_none() {
            let l = line.to_lowercase();
            if l.starts_with("supersedes:") {
                let v = line[11..].trim();
                if !v.is_empty() {
                    supersedes = Some(slug_of(v));
                }
            }
        }
    }
    DocMeta {
        kind,
        slug: slug.to_string(),
        title: title_override
            .map(str::to_string)
            .or(title)
            .unwrap_or_else(|| slug.to_string()),
        status: match kind {
            DocKind::Decision => Some(
                status_override
                    .map(|s| s.to_lowercase())
                    .or(status)
                    .unwrap_or_else(|| "recorded".to_string()),
            ),
            DocKind::Plan => status_override.map(|s| s.to_lowercase()).or(status),
        },
        supersedes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_conventions_detect_kind() {
        assert_eq!(
            path_kind("docs/adr/adr-001-storage.md"),
            Some((DocKind::Decision, "adr-001-storage".to_string()))
        );
        assert_eq!(
            path_kind("docs/decisions/use-rust.md"),
            Some((DocKind::Decision, "use-rust".to_string()))
        );
        assert_eq!(
            path_kind("ADR-002-Sync.md"),
            Some((DocKind::Decision, "adr-002-sync".to_string()))
        );
        assert_eq!(
            path_kind("docs/plans/twin-v1.md"),
            Some((DocKind::Plan, "twin-v1".to_string()))
        );
        assert_eq!(path_kind("docs/architecture.md"), None);
        assert_eq!(path_kind("docs/adr/notes.txt"), None);
    }

    #[test]
    fn adr_title_status_and_supersedes_parse() {
        let md = "# Use content addressing\n\nStatus: Accepted\nSupersedes: docs/adr/adr-000-files.md\n\nBody.\n";
        let meta = parse_doc("docs/adr/adr-001-storage.md", md).unwrap();
        assert_eq!(meta.title, "Use content addressing");
        assert_eq!(meta.status.as_deref(), Some("accepted"));
        assert_eq!(meta.supersedes.as_deref(), Some("adr-000-files"));
    }

    #[test]
    fn status_heading_convention_and_fallbacks() {
        let md = "# Choice\n\n## Status\n\nsuperseded\n";
        let meta = parse_doc("docs/adr/adr-003.md", md).unwrap();
        assert_eq!(meta.status.as_deref(), Some("superseded"));

        // No title heading, no status: fall back to slug + "recorded".
        let meta = parse_doc("docs/adr/adr-004-untitled.md", "just prose\n").unwrap();
        assert_eq!(meta.title, "adr-004-untitled");
        assert_eq!(meta.status.as_deref(), Some("recorded"));

        // Plans carry a status only when the document declares one.
        let meta = parse_doc("docs/plans/p1.md", "# The Plan\nStatus: draft\n").unwrap();
        assert_eq!(meta.status.as_deref(), Some("draft"));
        assert_eq!(meta.title, "The Plan");
        let meta = parse_doc("docs/plans/p2.md", "# Quiet Plan\n").unwrap();
        assert_eq!(meta.status, None, "no default status for plans");
    }

    #[test]
    fn explicit_parse_with_overrides() {
        let meta = parse_content(
            DocKind::Plan,
            "twin-v1",
            "# Twin v1 — Reflective Mode as the First Deliverable\n...",
            Some("Twin v1 plan"),
            None,
        );
        assert_eq!(meta.title, "Twin v1 plan");
        assert_eq!(meta.kind, DocKind::Plan);
    }
}
