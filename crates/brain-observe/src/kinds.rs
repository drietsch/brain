//! The artifact-kind registry: one merged view of what kinds exist.
//!
//! A kind's record is its template entity (ADR-003/008) — schema, capture
//! rules, placement, enforcement, rot policy, parser routing. This module
//! overlays the compiled defaults ([`crate::templates::DEFAULTS`]) with
//! the graph's observations (graph wins per property), so "built-ins are
//! pre-taught defaults" is literal: a store that never re-seeded still
//! knows the shipped kinds, and a store that edited a template sees its
//! own values everywhere.

use crate::templates::{self, CaptureRule};
use crate::twin::latest;
use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::{Index, MemIndex};
use brain_store::{Store, StoreError};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default)]
pub struct KindDef {
    /// The entity kind this record governs (`applies_to`).
    pub kind: String,
    /// Template slug (`adr`, `plan`, `runbook`, ...).
    pub slug: String,
    /// The graph template entity, when one exists in this store.
    pub template: Option<StableId>,
    pub requires: Vec<String>,
    pub capture: Vec<String>,
    pub fields: Vec<(String, String, Option<String>)>,
    /// graph_first | file_first | projection (default file_first).
    pub placement: String,
    /// Where files of this kind belong (globs).
    pub home: Vec<String>,
    /// Render path for graph-first/projection kinds; `{slug}` is filled.
    pub project_to: String,
    /// advisory (default) | enforced — the opt-in gate.
    pub enforce: String,
    /// none | info | warn (staleness severity), empty = code default.
    pub rot: String,
    /// doc.decision | doc.plan | agent | fields — capture routing.
    pub parser: String,
    /// Link predicates this kind participates in (advisory vocabulary).
    pub links: Vec<String>,
    /// Extra file extensions this kind's capture may ingest.
    pub extensions: Vec<String>,
    /// Scaffold.
    pub content: String,
    /// Current contract version (blake3 of requires + content).
    pub contract: String,
}

fn csv(s: &str) -> Vec<String> {
    s.split(',').map(str::trim).filter(|x| !x.is_empty()).map(str::to_string).collect()
}

impl KindDef {
    fn from_default(def: &templates::TemplateDef) -> KindDef {
        let mut kd = KindDef {
            kind: def.applies_to.to_string(),
            slug: def.slug.to_string(),
            template: None,
            requires: csv(def.requires),
            content: def.content.to_string(),
            placement: "file_first".to_string(),
            enforce: "advisory".to_string(),
            parser: "fields".to_string(),
            ..KindDef::default()
        };
        for (prop, value) in def.extra {
            kd.apply(prop, value);
        }
        kd.contract = templates::contract_b3(def.requires, def.content);
        kd
    }

    fn apply(&mut self, prop: &str, value: &str) {
        match prop {
            "requires" => self.requires = csv(value),
            "capture" => self.capture = csv(value),
            "fields" => self.fields = templates::parse_fields(value),
            "placement" => self.placement = value.to_string(),
            "home" => self.home = csv(value),
            "project_to" => self.project_to = value.to_string(),
            "enforce" => self.enforce = value.to_string(),
            "rot" => self.rot = value.to_string(),
            "parser" => self.parser = value.to_string(),
            "links" => self.links = csv(value),
            "extensions" => self.extensions = csv(value),
            "content" => self.content = value.to_string(),
            "contract_b3" => self.contract = value.to_string(),
            _ => {}
        }
    }

    /// The capture rule this kind contributes, when it captures paths.
    pub fn rule(&self) -> Option<CaptureRule> {
        if self.capture.is_empty() {
            return None;
        }
        Some(CaptureRule {
            kind: self.kind.clone(),
            patterns: self.capture.clone(),
            fields: self.fields.clone(),
        })
    }
}

/// The merged registry, keyed by entity kind. Compiled defaults form the
/// base; every graph template overlays its observed properties on top.
pub fn registry(
    store: &Store,
    index: &MemIndex,
) -> Result<BTreeMap<String, KindDef>, StoreError> {
    let mut out: BTreeMap<String, KindDef> = BTreeMap::new();
    for def in templates::DEFAULTS {
        let kd = KindDef::from_default(def);
        out.insert(kd.kind.clone(), kd);
    }
    let mut seen: BTreeSet<StableId> = BTreeSet::new();
    for node in index.entities_by_kind("template") {
        let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else { continue };
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(applies) = latest(index, store, &id, "applies_to")? else { continue };
        let entry = out.entry(applies.clone()).or_insert_with(|| KindDef {
            kind: applies.clone(),
            slug: labels.get("slug").cloned().unwrap_or_else(|| applies.clone()),
            placement: "file_first".to_string(),
            enforce: "advisory".to_string(),
            parser: "fields".to_string(),
            ..KindDef::default()
        });
        entry.template = Some(id.clone());
        for prop in [
            "requires",
            "capture",
            "fields",
            "placement",
            "home",
            "project_to",
            "enforce",
            "rot",
            "parser",
            "links",
            "extensions",
            "content",
            "contract_b3",
        ] {
            if let Some(value) = latest(index, store, &id, prop)? {
                entry.apply(prop, &value);
            }
        }
    }
    Ok(out)
}

/// The kinds whose artifacts are captured documents — the loop set for
/// staleness, conformance, association, and consolidation. Built-ins
/// first, then every registry kind that captures paths.
pub fn doc_kinds(
    store: &Store,
    index: &MemIndex,
) -> Result<Vec<String>, StoreError> {
    let builtin = ["decision", "plan", "skill", "agent_config"];
    let mut out: Vec<String> = builtin.iter().map(|s| s.to_string()).collect();
    for (kind, def) in registry(store, index)? {
        if !builtin.contains(&kind.as_str())
            && kind != "feature"
            && (!def.capture.is_empty() || def.placement == "graph_first")
        {
            out.push(kind);
        }
    }
    // Graph-taught templates without capture still count when they govern
    // captured artifacts (legacy taught kinds).
    for kind in templates::by_kind(store, index)?.keys() {
        if !out.contains(kind) && kind != "feature" {
            out.push(kind.clone());
        }
    }
    Ok(out)
}

/// Capture rules from the merged registry, most-specific pattern wins at
/// match time. Rules only route `fields`-parser kinds; decision/plan and
/// agent documents keep their richer code parsers.
pub fn capture_rules(reg: &BTreeMap<String, KindDef>) -> Vec<CaptureRule> {
    reg.values()
        .filter(|def| def.parser == "fields")
        .filter_map(KindDef::rule)
        .collect()
}

/// Pick the rule for a path: the matching pattern with the most literal
/// (non-wildcard) characters wins; ties break to the smallest kind name.
pub fn match_rule<'a>(rules: &'a [CaptureRule], rel_path: &str) -> Option<&'a CaptureRule> {
    let mut best: Option<(usize, &str, &CaptureRule)> = None;
    for rule in rules {
        for pattern in &rule.patterns {
            if !templates::glob_match(pattern, rel_path) {
                continue;
            }
            let literal = pattern.chars().filter(|c| !matches!(c, '*' | '?')).count();
            let better = match &best {
                None => true,
                Some((l, k, _)) => literal > *l || (literal == *l && rule.kind.as_str() < *k),
            };
            if better {
                best = Some((literal, &rule.kind, rule));
            }
        }
    }
    best.map(|(_, _, r)| r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain_index::replay;

    #[test]
    fn registry_overlays_graph_on_compiled_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();

        // Cold store, nothing seeded: compiled defaults still register.
        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        let reg = registry(&store, &index).unwrap();
        assert!(reg.contains_key("decision") && reg.contains_key("runbook"));
        assert_eq!(reg["plan"].placement, "graph_first");
        assert_eq!(reg["decision"].parser, "doc.decision");
        assert!(!reg["doc"].capture.is_empty());
        assert!(reg["decision"].template.is_none(), "no graph entity yet");

        // Seed + a local edit: the graph value wins for that property only.
        templates::seed(&store).unwrap();
        let sid = templates::template_sid("runbook");
        crate::twin::observe_src(
            &store,
            &sid,
            "enforce",
            "enforced",
            "agent",
            brain_store::now_ms(),
        )
        .unwrap();
        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        let reg = registry(&store, &index).unwrap();
        assert_eq!(reg["runbook"].enforce, "enforced", "graph wins");
        assert_eq!(reg["runbook"].placement, "file_first", "untouched props keep defaults");
        assert!(reg["runbook"].template.is_some());
        assert!(!reg["runbook"].contract.is_empty());
    }

    #[test]
    fn most_specific_pattern_wins() {
        let rules = vec![
            CaptureRule {
                kind: "doc".into(),
                patterns: vec!["README.md".into(), "docs/*.md".into()],
                fields: vec![],
            },
            CaptureRule {
                kind: "runbook".into(),
                patterns: vec!["docs/runbooks/**/*.md".into()],
                fields: vec![],
            },
        ];
        assert_eq!(match_rule(&rules, "docs/runbooks/deploy.md").unwrap().kind, "runbook");
        assert_eq!(match_rule(&rules, "docs/architecture.md").unwrap().kind, "doc");
        assert_eq!(match_rule(&rules, "README.md").unwrap().kind, "doc");
        assert!(match_rule(&rules, "src/main.rs").is_none());
    }
}
