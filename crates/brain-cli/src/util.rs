//! Utilities: the manual, benchmarks, and the small readers around the store.

use brain_core::object::Object;
use brain_index::{object_edges, Index};
use crate::support::*;

/// `brain man` — the manual, projected from the same registry as usage().
pub(crate) fn cmd_man(args: &[String]) -> Result<(), String> {
    let page = crate::manual::man_page();
    if let Some(i) = args.iter().position(|a| a == "--out") {
        let path = args.get(i + 1).ok_or("--out needs a path")?;
        std::fs::write(path, &page).map_err(|e| e.to_string())?;
        println!("wrote {path}");
        return Ok(());
    }
    if args.iter().any(|a| a == "--install") {
        let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
        let dir = std::path::Path::new(&home).join(".local/share/man/man1");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join("brain.1");
        std::fs::write(&path, &page).map_err(|e| e.to_string())?;
        println!("installed {} — try: man brain", path.display());
        return Ok(());
    }
    print!("{page}");
    Ok(())
}

/// `brain bench index` — the earn-adoption gate: honest numbers, cold
/// reference replay vs cortex warm open, plus a real query mix, with
/// answers verified identical before any timing is trusted.
pub(crate) fn cmd_bench(args: &[String]) -> Result<(), String> {
    if args.first().map(String::as_str) != Some("index") {
        return Err("usage: brain bench index [--prefix <p>]".to_string());
    }
    let prefix = args
        .iter()
        .position(|a| a == "--prefix")
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_else(|| "twin/self".to_string());
    let store = open_store()?;
    let objects = store.count_objects().map_err(|e| e.to_string())?;

    let t0 = std::time::Instant::now();
    let cold = brain_cortex::Cortex::open_ephemeral(&store).map_err(|e| e.to_string())?;
    let cold_time = t0.elapsed();

    // Ensure a checkpoint exists, then measure the warm path.
    brain_cortex::Cortex::open(&store)
        .and_then(|g| g.checkpoint().map(|_| ()))
        .map_err(|e| e.to_string())?;
    let t1 = std::time::Instant::now();
    let warm = brain_cortex::Cortex::open(&store).map_err(|e| e.to_string())?;
    let warm_time = t1.elapsed();

    // Correctness first: identical answers over real probes, or no bench.
    let mut sids = Vec::new();
    for (name, node) in store.namespace().map_err(|e| e.to_string())? {
        if name.strip_prefix(&format!("{prefix}/")).is_some() {
            if let Ok(Object::Entity { id, .. }) = store.get(&node) {
                sids.push(id);
            }
        }
    }
    if !brain_cortex::answers_match(
        &*cold,
        &*warm,
        &[],
        &sids,
        &["source_file", "decision", "test_run"],
        &["imports", "contains", "mentions"],
    ) {
        return Err("backends disagree — cortex does not earn adoption".to_string());
    }

    // A real query mix over the warm index — against a *fresh* store.
    //
    // The store caches objects for the life of the process, so running
    // this on the store the cold replay just walked would measure a fully
    // warm cache and report a number no real command can achieve. A
    // command opens a checkpoint and then reads the objects it needs; this
    // reproduces that.
    let query_store = open_store()?;
    let query_index = brain_cortex::Cortex::open(&query_store).map_err(|e| e.to_string())?;
    let t2 = std::time::Instant::now();
    let mut edges = 0usize;
    for sid in &sids {
        edges += query_index.relations_from(sid, "imports").len();
        edges += query_index.relations_to(sid, "imports").len();
        edges += query_index.relations_from(sid, "contains").len();
    }
    let ins = brain_observe::twin::insights_with(&query_store, &query_index, &prefix)
        .map_err(|e| e.to_string())?;
    let query_time = t2.elapsed();

    // And again on the same store, to show what the cache is worth.
    let t3 = std::time::Instant::now();
    let _ = brain_observe::twin::insights_with(&query_store, &query_index, &prefix)
        .map_err(|e| e.to_string())?;
    let requery_time = t3.elapsed();

    println!(
        "store: {objects} objects; probes: {} entities, {edges} edge answers",
        sids.len()
    );
    println!("cold replay (BRAIN_INDEX=mem behavior): {cold_time:?}");
    println!(
        "warm cortex open (delta {} event(s)):   {warm_time:?}  ({:.1}x faster)",
        warm.delta(),
        cold_time.as_secs_f64() / warm_time.as_secs_f64().max(1e-9)
    );
    println!("query mix (edges + full insights):      {query_time:?}  (cold store)");
    println!("the same query again (warm objects):    {requery_time:?}");
    let reads = query_store.reads();
    println!(
        "reads to answer it: {} served, {} went to bytes",
        reads.served, reads.from_disk
    );
    println!(
        "answers: identical across backends ({} files in insights)",
        ins.files
    );
    Ok(())
}

pub(crate) fn cmd_refs(args: &[String]) -> Result<(), String> {
    let arg = args.first().ok_or("usage: brain refs <name|b3:hash>")?;
    let store = open_store()?;
    let target = resolve_arg(&store, arg)?;
    let index = build_index(&store)?;
    let names = names_of(&store)?;
    let referrers = index.referrers(&target);
    if referrers.is_empty() {
        println!("nothing references {target:?}");
    } else {
        for id in referrers {
            println!("{}", describe(&store, &names, &id));
        }
    }
    Ok(())
}

pub(crate) fn cmd_evidence(args: &[String]) -> Result<(), String> {
    let arg = args.first().ok_or("usage: brain evidence <name|b3:hash>")?;
    let store = open_store()?;
    let target = resolve_arg(&store, arg)?;
    let index = build_index(&store)?;
    let evidence = index.evidence_for(&target);
    if evidence.is_empty() {
        println!("no evidence recorded for {target:?}");
        return Ok(());
    }
    for id in evidence {
        if let Object::Evidence {
            level,
            method,
            passed,
            detail,
            ..
        } = store.get(&id).map_err(|e| e.to_string())?
        {
            let mark = if passed { "pass" } else { "FAIL" };
            println!("{mark}  {level:?}  {method}  {detail}");
        }
    }
    Ok(())
}

pub(crate) fn cmd_deps(args: &[String]) -> Result<(), String> {
    let arg = args.first().ok_or("usage: brain deps <name|b3:hash>")?;
    let store = open_store()?;
    let target = resolve_arg(&store, arg)?;
    let obj = store.get(&target).map_err(|e| e.to_string())?;
    let names = names_of(&store)?;
    let edges = object_edges(&obj);
    if edges.is_empty() {
        println!("{target:?} references nothing");
    } else {
        for (kind, id) in edges {
            println!("{kind:?}  {}", describe(&store, &names, &id));
        }
    }
    Ok(())
}
