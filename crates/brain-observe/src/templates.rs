//! Templates: the deliverable contract, defined in the graph.
//!
//! What an ADR, a plan, a skill, or a "done" feature must contain is not
//! hardcoded knowledge or tribal convention — it is data in the graph:
//! `template` entities with `content` (a scaffold), `requires`
//! (machine-checkable fields), and `applies_to` (the entity kind governed).
//! Templates version through observations, evolve per store, and replicate
//! with `brain pull` — the team's working contract travels with the software.
//!
//! Conformance is *recorded, never enforced* in reflective mode: a document
//! missing its required fields gets a `conforms=false` observation that
//! surfaces in insights, not a rejection at capture time.

use crate::twin::{latest, observe_src};
use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::{replay, Index, MemIndex};
use brain_store::{now_ms, Store, StoreError};
use std::collections::{BTreeMap, BTreeSet};

pub struct TemplateDef {
    pub slug: &'static str,
    pub title: &'static str,
    /// Entity kind this template governs (decision, plan, skill, feature).
    pub applies_to: &'static str,
    /// Comma-separated machine-checkable required fields. For document
    /// kinds these are content checks; for `feature` they are the relation
    /// predicates of the definition of done.
    pub requires: &'static str,
    /// Markdown scaffold; `{{title}}` is filled at instantiation.
    pub content: &'static str,
}

/// The constitutional defaults, seeded at init and overridable per store
/// (a re-seed after local edits writes nothing that would regress them —
/// every observation is guarded, and newer local values win via `latest`).
pub const DEFAULTS: &[TemplateDef] = &[
    TemplateDef {
        slug: "adr",
        title: "Architecture decision record",
        applies_to: "decision",
        requires: "title,status",
        content: "# {{title}}\n\nStatus: proposed\n\n## Context\n\nWhat forces are at play?\n\n## Decision\n\nWhat was decided, stated actively.\n\n## Consequences\n\nWhat becomes easier, what becomes harder.\n",
    },
    TemplateDef {
        slug: "plan",
        title: "Implementation plan",
        applies_to: "plan",
        requires: "title",
        content: "# {{title}}\n\n## Context\n\nWhy this work exists.\n\n## Design\n\nWhat will be built, and how.\n\n## Verification\n\nHow we will know it worked.\n",
    },
    TemplateDef {
        slug: "skill",
        title: "Agent skill",
        applies_to: "skill",
        requires: "name,description",
        content: "---\nname: {{title}}\ndescription: When and why an agent should reach for this skill.\n---\n\n# {{title}}\n\nSteps, commands, and the judgment calls that matter.\n",
    },
    TemplateDef {
        slug: "dod",
        title: "Definition of done",
        applies_to: "feature",
        requires: "implemented_by,tested_by,decided_by,documented_in",
        content: "# Definition of done\n\nA feature counts as done when the graph shows, for its entity:\n\n- `implemented_by` -> at least one source file\n- `tested_by` -> at least one test file\n- `decided_by` -> a decision record (ADR)\n- `documented_in` -> documentation\n\nEvaluated by `brain done <prefix> <slug>`; the matrix is a rendered query,\nnot a spreadsheet.\n",
    },
];

pub fn template_sid(slug: &str) -> StableId {
    StableId::derive(&["template", slug])
}

/// Write the default templates into the graph (guarded: idempotent, and a
/// locally-edited template is not overwritten unless the default text
/// itself is what changed). Binds each under `brain/templates/<slug>`.
pub fn seed(store: &Store) -> Result<usize, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    let now = now_ms();
    let ns = store.namespace()?;
    let mut bindings = Vec::new();
    let mut written = 0;
    for def in DEFAULTS {
        let sid = template_sid(def.slug);
        let mut labels = BTreeMap::new();
        labels.insert("slug".to_string(), def.slug.to_string());
        labels.insert("title".to_string(), def.title.to_string());
        let node = store.put(&Object::Entity {
            id: sid.clone(),
            entity_kind: "template".to_string(),
            labels,
        })?;
        for (prop, value) in [
            ("content", def.content),
            ("requires", def.requires),
            ("applies_to", def.applies_to),
            ("title", def.title),
        ] {
            if latest(&index, store, &sid, prop)?.is_none() {
                observe_src(store, &sid, prop, value, "seed", now)?;
                written += 1;
            }
        }
        let name = format!("brain/templates/{}", def.slug);
        if !ns.contains_key(&name) {
            bindings.push((name, node));
        }
    }
    if !bindings.is_empty() {
        store.bind_many(bindings)?;
    }
    Ok(written)
}

/// Map of `applies_to` kind -> (template sid, required fields), read from
/// the graph. Empty when no templates have been seeded.
pub fn by_kind(
    store: &Store,
    index: &MemIndex,
) -> Result<BTreeMap<String, (StableId, Vec<String>)>, StoreError> {
    let mut out = BTreeMap::new();
    let mut seen: BTreeSet<StableId> = BTreeSet::new();
    for node in index.entities_by_kind("template") {
        let Ok(Object::Entity { id, .. }) = store.get(&node) else { continue };
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(applies) = latest(index, store, &id, "applies_to")? else { continue };
        let requires: Vec<String> = latest(index, store, &id, "requires")?
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        out.insert(applies, (id, requires));
    }
    Ok(out)
}

/// Which required fields are missing from a document's content? Checks are
/// deliberately shallow and honest: the machine-checkable minimum, not a
/// schema fighting the forgiving-parser philosophy.
pub fn check(content: &str, requires: &[String]) -> Vec<String> {
    let fm = crate::agents::frontmatter(content);
    requires.iter().filter(|r| !field_present(content, &fm, r)).cloned().collect()
}

fn field_present(content: &str, fm: &BTreeMap<String, String>, field: &str) -> bool {
    match field {
        "title" => content.lines().any(|l| l.trim().starts_with("# ")),
        "status" => content.lines().any(|l| {
            let t = l.trim().to_lowercase();
            (t.starts_with("status:") && t.len() > "status:".len()) || t == "## status"
        }),
        "name" => fm.contains_key("name"),
        "description" => fm.contains_key("description"),
        other => content.to_lowercase().contains(&format!("{other}:")),
    }
}

/// Fill a scaffold's placeholders.
pub fn instantiate(content: &str, title: &str) -> String {
    content.replace("{{title}}", title)
}

// ---------------------------------------------------------------------------
// Graph-defined capture rules: teach brain a new artifact type, no code
// ---------------------------------------------------------------------------

/// A capture rule declared on a template entity as data: which paths are
/// artifacts of its kind, and how to lift fields out of their text.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureRule {
    pub kind: String,
    pub patterns: Vec<String>,
    /// (property, extractor, optional arg) triples from the `fields` DSL.
    pub fields: Vec<(String, String, Option<String>)>,
}

/// All capture rules in the graph, read from `capture` / `fields`
/// observations on template entities. Kinds with a rule extend what the
/// twin auto-captures — the built-in detectors keep precedence.
pub fn capture_rules(store: &Store, index: &MemIndex) -> Result<Vec<CaptureRule>, StoreError> {
    let mut out = Vec::new();
    let mut seen: BTreeSet<StableId> = BTreeSet::new();
    for node in index.entities_by_kind("template") {
        let Ok(Object::Entity { id, .. }) = store.get(&node) else { continue };
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(kind) = latest(index, store, &id, "applies_to")? else { continue };
        let Some(capture) = latest(index, store, &id, "capture")? else { continue };
        let patterns: Vec<String> = capture
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        if patterns.is_empty() {
            continue;
        }
        let fields = parse_fields(&latest(index, store, &id, "fields")?.unwrap_or_default());
        out.push(CaptureRule { kind, patterns, fields });
    }
    Ok(out)
}

/// Parse the `fields` DSL: `prop=extractor[:arg]` comma-separated.
pub fn parse_fields(spec: &str) -> Vec<(String, String, Option<String>)> {
    spec.split(',')
        .filter_map(|part| {
            let (prop, rest) = part.trim().split_once('=')?;
            let (extractor, arg) = match rest.split_once(':') {
                Some((e, a)) => (e, Some(a.trim().to_string())),
                None => (rest, None),
            };
            let prop = prop.trim();
            if prop.is_empty() || extractor.trim().is_empty() {
                return None;
            }
            Some((prop.to_string(), extractor.trim().to_string(), arg))
        })
        .collect()
}

impl CaptureRule {
    pub fn matches(&self, rel_path: &str) -> bool {
        self.patterns.iter().any(|p| glob_match(p, rel_path))
    }

    /// Extract this rule's fields from a document. Fields whose extractor
    /// finds nothing are simply absent — capture is forgiving; the
    /// `requires` contract is what flags gaps, as conformance.
    pub fn extract(&self, content: &str, slug: &str) -> Vec<(String, String)> {
        let fm = crate::agents::frontmatter(content);
        let mut out = Vec::new();
        for (prop, extractor, arg) in &self.fields {
            let value = match extractor.as_str() {
                "heading" => content
                    .lines()
                    .find_map(|l| l.trim().strip_prefix("# ").map(|t| t.trim().to_string())),
                "line" => {
                    // `Key:` line; default key = property with first letter
                    // upper-cased (service -> "Service:").
                    let key = arg.clone().unwrap_or_else(|| {
                        let mut c = prop.chars();
                        c.next()
                            .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                            .unwrap_or_default()
                    });
                    let needle = format!("{}:", key.to_lowercase());
                    content.lines().find_map(|l| {
                        let t = l.trim();
                        t.to_lowercase()
                            .strip_prefix(&needle)
                            .map(|_| t[needle.len()..].trim().to_string())
                            .filter(|v| !v.is_empty())
                    })
                }
                "frontmatter" => fm.get(arg.as_deref().unwrap_or(prop)).cloned(),
                "slug" => Some(slug.to_string()),
                _ => None,
            };
            if let Some(v) = value {
                out.push((prop.clone(), v));
            }
        }
        out
    }
}

/// Minimal glob: `*` matches within a path segment, `**` across segments,
/// `?` one non-slash character. No dependency, no surprises.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    fn inner(p: &[u8], s: &[u8]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (None, Some(_)) => false,
            (Some(b'*'), _) if p.get(1) == Some(&b'*') => {
                // `**`: swallow any run (including slashes); a following
                // `/` may match zero directories.
                let rest = if p.get(2) == Some(&b'/') { &p[3..] } else { &p[2..] };
                (0..=s.len()).any(|i| inner(rest, &s[i..]))
            }
            (Some(b'*'), _) => {
                let rest = &p[1..];
                (0..=s.len())
                    .take_while(|i| *i == 0 || s[i - 1] != b'/')
                    .any(|i| inner(rest, &s[i..]))
            }
            (Some(b'?'), Some(c)) if *c != b'/' => inner(&p[1..], &s[1..]),
            (Some(a), Some(b)) if a == b => inner(&p[1..], &s[1..]),
            _ => false,
        }
    }
    inner(pattern.as_bytes(), path.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_idempotent_and_queryable() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let n = seed(&store).unwrap();
        assert!(n >= 16, "four templates, four observations each: {n}");
        let before = store.count_objects().unwrap();
        assert_eq!(seed(&store).unwrap(), 0, "re-seed writes nothing");
        assert_eq!(store.count_objects().unwrap(), before);

        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        let map = by_kind(&store, &index).unwrap();
        assert_eq!(map.get("decision").unwrap().1, vec!["title", "status"]);
        assert!(map.get("feature").unwrap().1.contains(&"tested_by".to_string()));
        assert!(store.resolve("brain/templates/adr").unwrap().is_some());
    }

    #[test]
    fn local_template_edits_survive_reseed() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        seed(&store).unwrap();
        // A store tightens its ADR contract: supersedes becomes required.
        let sid = template_sid("adr");
        observe_src(&store, &sid, "requires", "title,status,supersedes", "agent", now_ms())
            .unwrap();
        seed(&store).unwrap();
        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        let map = by_kind(&store, &index).unwrap();
        assert_eq!(map.get("decision").unwrap().1.len(), 3, "local override wins");
    }

    #[test]
    fn glob_matches_segments_and_double_star() {
        assert!(glob_match("docs/runbooks/*.md", "docs/runbooks/deploy.md"));
        assert!(!glob_match("docs/runbooks/*.md", "docs/runbooks/sub/deploy.md"));
        assert!(glob_match("runbooks/**/*.md", "runbooks/a/b/deploy.md"));
        assert!(glob_match("runbooks/**/*.md", "runbooks/deploy.md"), "** may match zero dirs");
        assert!(glob_match("**/*.rfc", "any/depth/x.rfc"));
        assert!(glob_match("incident-????.md", "incident-0042.md"));
        assert!(!glob_match("incident-????.md", "incident-42.md"));
        assert!(!glob_match("*.md", "docs/x.md"), "* does not cross slashes");
    }

    #[test]
    fn capture_fields_extract_per_dsl() {
        let fields = parse_fields("title=heading, service=line, owner=frontmatter, id=slug, sev=line:Severity");
        assert_eq!(fields.len(), 5);
        let rule = CaptureRule {
            kind: "runbook".into(),
            patterns: vec!["docs/runbooks/*.md".into()],
            fields,
        };
        assert!(rule.matches("docs/runbooks/deploy.md"));
        let md = "---\nowner: platform-team\n---\n\n# Deploy safely\n\nService: checkout\nSeverity: high\n";
        let got = rule.extract(md, "deploy");
        let get = |k: &str| got.iter().find(|(p, _)| p == k).map(|(_, v)| v.as_str());
        assert_eq!(get("title"), Some("Deploy safely"));
        assert_eq!(get("service"), Some("checkout"));
        assert_eq!(get("owner"), Some("platform-team"));
        assert_eq!(get("id"), Some("deploy"));
        assert_eq!(get("sev"), Some("high"));
        // Missing fields are absent, not errors — conformance flags gaps.
        assert!(rule.extract("no structure here\n", "x").iter().all(|(p, _)| p == "id"));
    }

    #[test]
    fn capture_rules_read_from_graph_observations() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        seed(&store).unwrap();
        // Defaults declare no capture rules (built-in detectors own those
        // kinds); a custom template becomes a rule via two observations.
        let sid = template_sid("runbook");
        store
            .put(&Object::Entity {
                id: sid.clone(),
                entity_kind: "template".to_string(),
                labels: BTreeMap::new(),
            })
            .unwrap();
        let now = now_ms();
        observe_src(&store, &sid, "applies_to", "runbook", "agent", now).unwrap();
        observe_src(&store, &sid, "capture", "docs/runbooks/*.md", "agent", now).unwrap();
        observe_src(&store, &sid, "fields", "title=heading, service=line", "agent", now).unwrap();
        observe_src(&store, &sid, "requires", "title,service", "agent", now).unwrap();

        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        let rules = capture_rules(&store, &index).unwrap();
        assert_eq!(rules.len(), 1, "only the runbook template captures: {rules:?}");
        assert_eq!(rules[0].kind, "runbook");
        assert!(rules[0].matches("docs/runbooks/deploy.md"));
    }

    #[test]
    fn conformance_checks_are_shallow_and_honest() {
        let req = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert!(check("# Title\n\nStatus: accepted\n", &req(&["title", "status"])).is_empty());
        assert_eq!(check("prose only\n", &req(&["title", "status"])), vec!["title", "status"]);
        assert_eq!(check("# T\n\nStatus:\n", &req(&["status"])), vec!["status"]);
        assert!(check("# T\n\n## Status\n\naccepted\n", &req(&["status"])).is_empty());
        assert!(check("---\nname: x\ndescription: y\n---\n", &req(&["name", "description"]))
            .is_empty());
        assert_eq!(instantiate("# {{title}}\n", "Do the thing"), "# Do the thing\n");
    }
}
