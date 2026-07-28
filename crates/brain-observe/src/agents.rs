//! Parsing for the agent-operating layer: skills and agent configuration.
//!
//! Agentically-built software carries files that configure the agents that
//! build it — skills (`SKILL.md`), instruction files (`CLAUDE.md`,
//! `AGENTS.md`, `.cursorrules`, ...), subagent and command definitions,
//! settings and MCP wiring. The twin captures them as first-class entities
//! so the *how it is built* persists beside the *what* and the *why*.
//!
//! Pure functions only; recording into the graph lives in [`crate::twin`].

use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentDocKind {
    /// A skill: a named, reusable instruction package (`SKILL.md`).
    Skill,
    /// Any other agent configuration: instructions, subagents, commands,
    /// settings, MCP wiring, editor rules.
    Config,
}

impl AgentDocKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentDocKind::Skill => "skill",
            AgentDocKind::Config => "agent_config",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentDoc {
    pub kind: AgentDocKind,
    pub slug: String,
    /// Which agent family the file configures: claude, codex, cursor,
    /// copilot, gemini, or generic (cross-agent conventions like AGENTS.md).
    pub agent: String,
    /// What the file does: skill, subagent, command, instructions,
    /// settings, mcp, rules.
    pub role: String,
    pub name: String,
    pub description: Option<String>,
}

/// Does this path follow an agent skill/configuration convention?
/// Returns (kind, slug, agent, role). Path-only, no content read.
pub fn path_agent_doc(rel_path: &str) -> Option<(AgentDocKind, String, String, String)> {
    let file = rel_path.rsplit('/').next().unwrap_or(rel_path);
    let claude = |s: &str, r: &str| {
        (
            AgentDocKind::Config,
            s.to_string(),
            "claude".into(),
            r.into(),
        )
    };

    // Skills: any <name>/SKILL.md — the skill's identity is its directory.
    if file == "SKILL.md" {
        let slug = rel_path
            .strip_suffix("/SKILL.md")
            .and_then(|d| d.rsplit('/').next())
            .unwrap_or("skill")
            .to_lowercase();
        return Some((AgentDocKind::Skill, slug, "claude".into(), "skill".into()));
    }
    // Instruction files, by exact filename (any depth — slug keeps the path
    // so a nested CLAUDE.md stays distinct from the root one).
    let path_slug = rel_path.to_lowercase();
    match file {
        "CLAUDE.md" => return Some(claude(&path_slug, "instructions")),
        "AGENTS.md" => {
            let agent = if rel_path.contains(".codex/") {
                "codex"
            } else {
                "generic"
            };
            return Some((
                AgentDocKind::Config,
                path_slug,
                agent.into(),
                "instructions".into(),
            ));
        }
        "GEMINI.md" => {
            return Some((
                AgentDocKind::Config,
                path_slug,
                "gemini".into(),
                "instructions".into(),
            ))
        }
        ".cursorrules" => {
            return Some((
                AgentDocKind::Config,
                path_slug,
                "cursor".into(),
                "rules".into(),
            ))
        }
        ".mcp.json" => return Some(claude(&path_slug, "mcp")),
        _ => {}
    }
    if rel_path == ".github/copilot-instructions.md" {
        return Some((
            AgentDocKind::Config,
            path_slug,
            "copilot".into(),
            "instructions".into(),
        ));
    }
    if rel_path.contains(".cursor/rules/") && rel_path.ends_with(".mdc") {
        return Some((
            AgentDocKind::Config,
            path_slug,
            "cursor".into(),
            "rules".into(),
        ));
    }
    // Inside a .claude directory: subagents, commands, settings.
    if let Some(rest) = rel_path.split(".claude/").nth(1) {
        if let Some(stem) = rest
            .strip_prefix("agents/")
            .and_then(|f| f.strip_suffix(".md"))
        {
            return Some(claude(&stem.to_lowercase(), "subagent"));
        }
        if let Some(stem) = rest
            .strip_prefix("commands/")
            .and_then(|f| f.strip_suffix(".md"))
        {
            return Some(claude(&stem.to_lowercase(), "command"));
        }
        if rest == "settings.json" || rest == "settings.local.json" {
            return Some(claude(&path_slug, "settings"));
        }
    }
    if rel_path.contains(".codex/") {
        return Some((
            AgentDocKind::Config,
            path_slug,
            "codex".into(),
            "settings".into(),
        ));
    }
    None
}

/// Parse an agent document whose kind was detected from its path.
pub fn parse_agent_doc(rel_path: &str, content: &str) -> Option<AgentDoc> {
    let (kind, slug, agent, role) = path_agent_doc(rel_path)?;
    Some(parse_agent_content(kind, &slug, &agent, &role, content))
}

/// Parse content as an agent document of known shape — the explicit-add
/// path for files outside the observed repo (e.g. `~/.claude/skills`).
pub fn parse_agent_content(
    kind: AgentDocKind,
    slug: &str,
    agent: &str,
    role: &str,
    content: &str,
) -> AgentDoc {
    let fm = frontmatter(content);
    AgentDoc {
        kind,
        slug: slug.to_string(),
        agent: agent.to_string(),
        role: role.to_string(),
        name: fm.get("name").cloned().unwrap_or_else(|| slug.to_string()),
        description: fm.get("description").cloned(),
    }
}

/// YAML-ish frontmatter: `key: value` lines between a leading `---` pair.
/// Forgiving by design — enough for SKILL.md and subagent definitions.
pub fn frontmatter(content: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut lines = content.lines();
    if lines.next().map(str::trim) != Some("---") {
        return out;
    }
    for line in lines {
        let t = line.trim();
        if t == "---" {
            break;
        }
        if let Some((k, v)) = t.split_once(':') {
            let (k, v) = (k.trim().to_lowercase(), v.trim());
            if !k.is_empty() && !v.is_empty() {
                out.insert(k, v.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_and_instruction_conventions_detect() {
        let (kind, slug, agent, role) = path_agent_doc(".claude/skills/deploy/SKILL.md").unwrap();
        assert_eq!(
            (kind, slug.as_str(), agent.as_str(), role.as_str()),
            (AgentDocKind::Skill, "deploy", "claude", "skill")
        );
        // Plugin-style skills without .claude/ still count.
        assert_eq!(
            path_agent_doc("skills/review/SKILL.md").unwrap().1,
            "review"
        );

        assert_eq!(
            path_agent_doc("CLAUDE.md").unwrap(),
            (
                AgentDocKind::Config,
                "claude.md".into(),
                "claude".into(),
                "instructions".into()
            )
        );
        // Nested instruction files keep distinct identities.
        assert_eq!(
            path_agent_doc("crates/core/CLAUDE.md").unwrap().1,
            "crates/core/claude.md"
        );
        assert_eq!(path_agent_doc("AGENTS.md").unwrap().2, "generic");
        assert_eq!(path_agent_doc("GEMINI.md").unwrap().2, "gemini");
        assert_eq!(path_agent_doc(".cursorrules").unwrap().3, "rules");
        assert_eq!(
            path_agent_doc(".cursor/rules/style.mdc").unwrap().2,
            "cursor"
        );
        assert_eq!(
            path_agent_doc(".github/copilot-instructions.md").unwrap().2,
            "copilot"
        );
        assert_eq!(path_agent_doc(".mcp.json").unwrap().3, "mcp");
        assert_eq!(path_agent_doc(".codex/config.toml").unwrap().2, "codex");
        assert_eq!(path_agent_doc("src/main.rs"), None);
        assert_eq!(path_agent_doc("docs/readme.md"), None);
    }

    #[test]
    fn claude_dir_subagents_commands_settings_detect() {
        let (_, slug, _, role) = path_agent_doc(".claude/agents/reviewer.md").unwrap();
        assert_eq!((slug.as_str(), role.as_str()), ("reviewer", "subagent"));
        let (_, slug, _, role) = path_agent_doc(".claude/commands/deploy.md").unwrap();
        assert_eq!((slug.as_str(), role.as_str()), ("deploy", "command"));
        assert_eq!(
            path_agent_doc(".claude/settings.json").unwrap().3,
            "settings"
        );
        assert_eq!(
            path_agent_doc(".claude/settings.local.json").unwrap().3,
            "settings"
        );
        // Out-of-repo style absolute-ish paths work through the same rules.
        assert_eq!(
            path_agent_doc("root/.claude/agents/fixer.md").unwrap().1,
            "fixer"
        );
    }

    #[test]
    fn frontmatter_supplies_name_and_description() {
        let md =
            "---\nname: deploy\ndescription: Ship the thing safely\n---\n\n# Deploy\nsteps...\n";
        let doc = parse_agent_doc(".claude/skills/deploy/SKILL.md", md).unwrap();
        assert_eq!(doc.name, "deploy");
        assert_eq!(doc.description.as_deref(), Some("Ship the thing safely"));

        // No frontmatter: name falls back to the slug.
        let doc = parse_agent_doc("CLAUDE.md", "# Project notes\n").unwrap();
        assert_eq!(doc.name, "claude.md");
        assert_eq!(doc.description, None);
    }
}
