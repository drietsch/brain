//! The store itself: create one, read what it holds, move truth between stores.

use crate::support::*;

pub(crate) fn print_sync_report(report: &brain_store::sync::SyncReport) {
    println!(
        "objects: {} copied, {} already present",
        report.objects_copied, report.objects_present
    );
    println!(
        "names:   {} added, {} agreed",
        report.names_added, report.names_agreed
    );
    for (name, kept, incoming) in &report.conflicts {
        println!(
            "CONFLICT {name}: kept {kept:?}, source's {incoming:?} bound as sync-conflict/{name}"
        );
    }
}

pub(crate) fn cmd_init() -> Result<(), String> {
    let store = open_store()?;
    let seeded = brain_observe::templates::seed(&store).map_err(|e| e.to_string())?;
    println!("store ready at {}", store.root().display());
    if seeded > 0 {
        println!(
            "seeded {} deliverable templates under brain/templates/",
            brain_observe::templates::DEFAULTS.len()
        );
    }
    Ok(())
}

pub(crate) fn cmd_status() -> Result<(), String> {
    let store = open_store()?;
    let head = store.head().map_err(|e| e.to_string())?;
    println!(
        "objects:    {}",
        store.count_objects().map_err(|e| e.to_string())?
    );
    println!(
        "names:      {}",
        store.namespace().map_err(|e| e.to_string())?.len()
    );
    println!(
        "head:       {}",
        head.map(|h| h.to_string())
            .unwrap_or_else(|| "(none)".to_string())
    );
    println!(
        "history:    {} namespace step(s)",
        store.namespace_history().map_err(|e| e.to_string())?.len()
    );
    println!(
        "intents:    {}",
        store.intents().summary().map_err(|e| e.to_string())?
    );
    Ok(())
}

pub(crate) fn cmd_names() -> Result<(), String> {
    let store = open_store()?;
    for (name, id) in store.namespace().map_err(|e| e.to_string())? {
        println!("{name}  ->  {id}");
    }
    Ok(())
}

pub(crate) fn cmd_sync(args: &[String], pull: bool) -> Result<(), String> {
    let other_root = args.first().ok_or("usage: brain pull|push <store-root>")?;
    let local = open_store()?;
    let other = open_existing_store(other_root)?;
    let report = if pull {
        brain_store::sync::pull(&local, &other)
    } else {
        brain_store::sync::pull(&other, &local)
    }
    .map_err(|e| e.to_string())?;
    print_sync_report(&report);
    Ok(())
}
