//! Template fitness: the brain learns which contracts work.
//!
//! Every captured or authored artifact records the `template_b3` that
//! judged it (version-precise conformance), its earliest `conforms` and
//! `missing` observations (did agents produce it right on the first
//! try?), and its lifecycle timeline (did the artifact reach a good end?).
//! Fitness folds those into per-version rates — computed at query time,
//! integer arithmetic only, never persisted (ADR-009). `evolve` turns the
//! findings into a proposed next version; applying it is an explicit act
//! that bumps `contract_b3` and opens the next measurement window, so
//! versions stay comparable across brain generations.

use crate::lifecycle;
use crate::twin::{latest, latest_at, Severity};
use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::{Index, MemIndex};
use brain_store::{Store, StoreError};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Default)]
pub struct VersionFitness {
    /// contract_b3 of this version.
    pub contract: String,
    pub current: bool,
    pub artifacts: usize,
    /// Conforming at first capture / artifacts with first-capture data.
    pub first_conform: (usize, usize),
    /// Required field -> how many artifacts were missing it at first capture.
    pub missing: BTreeMap<String, usize>,
    /// Lifecycle state -> count, judged now.
    pub outcomes: BTreeMap<String, usize>,
    /// Artifacts of this version currently stale.
    pub stale_now: usize,
}

#[derive(Debug)]
pub struct TemplateFitness {
    pub slug: String,
    pub kind: String,
    pub enforce: String,
    pub versions: Vec<VersionFitness>,
    pub verdicts: Vec<String>,
}

/// Earliest observation (at, value) of a property on a subject.
fn earliest(
    index: &MemIndex,
    store: &Store,
    subject: &StableId,
    property: &str,
) -> Result<Option<(u64, String)>, StoreError> {
    let mut best: Option<(u64, String)> = None;
    for id in index.observations_of(subject) {
        if let Object::Observation { property: p, value, observed_at_ms, .. } = store.get(&id)? {
            if p == property && best.as_ref().is_none_or(|(b, _)| observed_at_ms < *b) {
                best = Some((observed_at_ms, value));
            }
        }
    }
    Ok(best)
}

/// Fitness for one template slug, or every registered one.
pub fn fitness(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    only_slug: Option<&str>,
) -> Result<Vec<TemplateFitness>, StoreError> {
    let registry = crate::kinds::registry(store, index)?;
    let ins = crate::twin::insights_with(store, index, prefix)?;
    let stale_slugs: BTreeSet<(String, String)> = ins
        .stale_docs
        .iter()
        .filter(|d| d.severity == Severity::Warn || d.severity == Severity::Info)
        .map(|d| (d.kind.clone(), d.slug.clone()))
        .collect();
    let mut out = Vec::new();

    for (kind, def) in &registry {
        if only_slug.is_some_and(|s| s != def.slug) {
            continue;
        }
        // Group this kind's artifacts by the contract that first judged them.
        let mut groups: BTreeMap<String, Vec<StableId>> = BTreeMap::new();
        let mut seen = BTreeSet::new();
        for node in index.entities_by_kind(kind) {
            let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else { continue };
            if labels.get("prefix").map(String::as_str) != Some(prefix)
                || !seen.insert(id.clone())
            {
                continue;
            }
            let Some((_, contract)) = earliest(index, store, &id, "template_b3")? else {
                continue; // judged by no version: pre-registry artifact
            };
            groups.entry(contract).or_default().push(id);
        }
        if groups.is_empty() && only_slug.is_none() {
            continue;
        }

        let mut versions = Vec::new();
        for (contract, artifacts) in &groups {
            let mut v = VersionFitness {
                contract: contract.clone(),
                current: *contract == def.contract,
                artifacts: artifacts.len(),
                ..VersionFitness::default()
            };
            for sid in artifacts {
                if let Some((_, conforms)) = earliest(index, store, sid, "conforms")? {
                    v.first_conform.1 += 1;
                    if conforms == "true" {
                        v.first_conform.0 += 1;
                    }
                }
                if let Some((_, missing)) = earliest(index, store, sid, "missing")? {
                    for field in missing.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                        *v.missing.entry(field.to_string()).or_default() += 1;
                    }
                }
                let (state, _) = lifecycle::of(index, store, sid)?;
                *v.outcomes.entry(state.as_str().to_string()).or_default() += 1;
                // Stale-now via the (kind, slug) set from insights.
                for node in index.entity_nodes(sid) {
                    if let Ok(Object::Entity { labels, .. }) = store.get(&node) {
                        if let Some(s) = labels.get("slug") {
                            if stale_slugs.contains(&(kind.clone(), s.clone())) {
                                v.stale_now += 1;
                            }
                        }
                        break;
                    }
                }
            }
            versions.push(v);
        }
        // Newest version (the current contract) first.
        versions.sort_by(|a, b| b.current.cmp(&a.current).then(a.contract.cmp(&b.contract)));

        let verdicts = judge(def, &versions);
        out.push(TemplateFitness {
            slug: def.slug.clone(),
            kind: kind.clone(),
            enforce: def.enforce.clone(),
            versions,
            verdicts,
        });
    }
    Ok(out)
}

/// Deterministic, integer-threshold findings.
fn judge(def: &crate::kinds::KindDef, versions: &[VersionFitness]) -> Vec<String> {
    let mut verdicts = Vec::new();
    let Some(current) = versions.iter().find(|v| v.current) else {
        return verdicts;
    };
    let (ok, total) = current.first_conform;
    if total >= 2 {
        for field in &def.requires {
            let missed = current.missing.get(field).copied().unwrap_or(0);
            if missed * 2 >= total {
                verdicts.push(format!(
                    "'{field}' missed in {missed}/{total} first captures — demote it (evolve) or make the scaffold carry it"
                ));
            } else if missed * 10 <= total && total >= 5 && def.enforce == "advisory" {
                verdicts.push(format!(
                    "'{field}' almost always present ({}/{total}) — consider `brain template set {} --enforce enforced`",
                    total - missed,
                    def.slug
                ));
            }
        }
        if ok * 2 < total {
            verdicts.push(format!(
                "first-capture conformance {ok}/{total} — agents fight this contract; simplify or scaffold harder"
            ));
        }
    }
    let done = current.outcomes.get("done").copied().unwrap_or(0);
    let abandoned = current.outcomes.get("abandoned").copied().unwrap_or(0);
    if done + abandoned >= 2 && abandoned * 2 >= done + abandoned {
        verdicts.push(format!(
            "{abandoned} of {} concluded artifacts were abandoned — the kind's scope may be wrong",
            done + abandoned
        ));
    }
    if current.artifacts > 0 && current.stale_now * 2 >= current.artifacts {
        verdicts.push(format!(
            "{}/{} currently stale — the kind may describe things that change faster than anyone re-reads",
            current.stale_now, current.artifacts
        ));
    }
    verdicts
}

#[derive(Debug)]
pub struct Evolution {
    /// Fields to demote from `requires` (missed in >= half of first captures).
    pub demote: Vec<String>,
    pub new_requires: Vec<String>,
    pub new_recommended: Vec<String>,
}

/// Propose the next version from fitness. Deterministic; returns `None`
/// when the evidence suggests nothing.
pub fn evolve(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    slug: &str,
) -> Result<Option<Evolution>, StoreError> {
    let all = fitness(store, index, prefix, Some(slug))?;
    let Some(tf) = all.first() else { return Ok(None) };
    let Some(current) = tf.versions.iter().find(|v| v.current) else { return Ok(None) };
    let (_, total) = current.first_conform;
    if total < 2 {
        return Ok(None);
    }
    let registry = crate::kinds::registry(store, index)?;
    let Some(def) = registry.values().find(|d| d.slug == slug) else { return Ok(None) };
    let demote: Vec<String> = def
        .requires
        .iter()
        .filter(|f| current.missing.get(*f).copied().unwrap_or(0) * 2 >= total)
        .cloned()
        .collect();
    if demote.is_empty() {
        return Ok(None);
    }
    let new_requires: Vec<String> =
        def.requires.iter().filter(|f| !demote.contains(f)).cloned().collect();
    let mut new_recommended: Vec<String> = Vec::new();
    if let Some(tmpl) = &def.template {
        if let Some(existing) = latest(index, store, tmpl, "recommended")? {
            new_recommended
                .extend(existing.split(',').map(str::trim).map(str::to_string));
        }
    }
    for f in &demote {
        if !new_recommended.contains(f) {
            new_recommended.push(f.clone());
        }
    }
    Ok(Some(Evolution { demote, new_requires, new_recommended }))
}

/// Apply an evolution: guarded writes of the new `requires` and
/// `recommended`, then a fresh `contract_b3` — the next measurement
/// window opens; old artifacts keep the version that judged them.
pub fn apply_evolution(
    store: &Store,
    index: &MemIndex,
    slug: &str,
    ev: &Evolution,
) -> Result<(), StoreError> {
    let sid = crate::templates::template_sid(slug);
    let now = brain_store::now_ms();
    let requires = ev.new_requires.join(",");
    let recommended = ev.new_recommended.join(",");
    for (prop, value) in [("requires", &requires), ("recommended", &recommended)] {
        if latest(index, store, &sid, prop)?.as_deref() != Some(value.as_str()) {
            crate::twin::observe_src(store, &sid, prop, value, "agent", now)?;
        }
    }
    let content = latest(index, store, &sid, "content")?.unwrap_or_default();
    crate::templates::stamp_contract(store, index, &sid, &requires, &content, "agent", now)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain_index::replay;
    use std::fs;

    fn fresh_index(store: &Store) -> MemIndex {
        let mut index = MemIndex::new();
        replay(store, &mut index).unwrap();
        index
    }

    #[test]
    fn fitness_measures_first_capture_conformance_and_evolve_demotes() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("docs/runbooks")).unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        crate::templates::seed(&store).unwrap();
        // Tighten the runbook contract: require service too.
        let tmpl = crate::templates::template_sid("runbook");
        crate::twin::observe_src(
            &store,
            &tmpl,
            "requires",
            "title,service",
            "agent",
            brain_store::now_ms(),
        )
        .unwrap();
        {
            let index = fresh_index(&store);
            let content = latest(&index, &store, &tmpl, "content").unwrap().unwrap();
            crate::templates::stamp_contract(
                &store,
                &index,
                &tmpl,
                "title,service",
                &content,
                "agent",
                brain_store::now_ms(),
            )
            .unwrap();
        }

        // Three runbooks; two keep forgetting the Service line.
        fs::write(src.path().join("docs/runbooks/a.md"), "# A\n\nService: x\nsteps\n").unwrap();
        fs::write(src.path().join("docs/runbooks/b.md"), "# B\n\nsteps only\n").unwrap();
        fs::write(src.path().join("docs/runbooks/c.md"), "# C\n\nmore steps\n").unwrap();
        crate::twin::refresh(&store, src.path(), "twin/app").unwrap();

        let index = fresh_index(&store);
        let all = fitness(&store, &index, "twin/app", Some("runbook")).unwrap();
        assert_eq!(all.len(), 1);
        let current = all[0].versions.iter().find(|v| v.current).unwrap();
        assert_eq!(current.artifacts, 3);
        assert_eq!(current.first_conform, (1, 3), "one of three conformed first try");
        assert_eq!(current.missing.get("service").copied(), Some(2));
        assert!(
            all[0].verdicts.iter().any(|v| v.contains("'service' missed in 2/3")),
            "{:?}",
            all[0].verdicts
        );

        // Evolve: demote the field agents keep skipping.
        let ev = evolve(&store, &index, "twin/app", "runbook").unwrap().unwrap();
        assert_eq!(ev.demote, vec!["service".to_string()]);
        assert_eq!(ev.new_requires, vec!["title".to_string()]);
        let old_contract = latest(&index, &store, &tmpl, "contract_b3").unwrap().unwrap();
        apply_evolution(&store, &index, "runbook", &ev).unwrap();
        let index = fresh_index(&store);
        assert_eq!(
            latest(&index, &store, &tmpl, "requires").unwrap().as_deref(),
            Some("title")
        );
        assert_eq!(
            latest(&index, &store, &tmpl, "recommended").unwrap().as_deref(),
            Some("service")
        );
        let new_contract = latest(&index, &store, &tmpl, "contract_b3").unwrap().unwrap();
        assert_ne!(old_contract, new_contract, "a new measurement window opened");

        // Old artifacts keep the version that judged them.
        let b = StableId::derive(&["runbook", "twin/app", "b"]);
        assert_eq!(
            latest(&index, &store, &b, "template_b3").unwrap().unwrap(),
            old_contract
        );
    }
}
