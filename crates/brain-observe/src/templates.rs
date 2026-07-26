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
