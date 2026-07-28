//! Held graph state: one `Store` and one warm `Cortex`, rebuilt only when
//! the event log actually grows.
//!
//! The previous implementation re-opened the store and re-scanned the whole
//! event log on every request — a dossier click cost roughly nine full
//! passes over every object in the graph. Here the index is held, and
//! freshness is a `stat` on `events.jsonl`: the log is append-only, so a
//! changed byte length is exactly the signal that the graph advanced.

use crate::dto::Snapshot;
use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_observe::attention::{self, Attention};
use brain_observe::coherence::{self, Finding};
use brain_observe::fitness::{self, TemplateFitness};
use brain_observe::kinds::{self, KindDef};
use brain_observe::twin::{self, Insights};
use brain_store::{now_ms, Store};
use cortex::Cortex;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct Config {
    pub store_root: PathBuf,
    /// Workspace root for file-backed bodies. Bytes are only ever read
    /// through a graph-recorded relative path that resolves inside it.
    pub content_root: PathBuf,
    pub prefix: String,
    pub bind: String,
    pub port: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            store_root: PathBuf::from(".brain"),
            content_root: PathBuf::from("."),
            prefix: "twin/self".to_string(),
            bind: "127.0.0.1".to_string(),
            port: 0,
        }
    }
}

/// One row of recorded activity, in event-log order.
#[derive(Debug, Clone)]
pub struct EventRow {
    pub at_ms: u64,
    pub subject: Option<StableId>,
    pub source: String,
    pub payload: EventPayload,
}

#[derive(Debug, Clone)]
pub enum EventPayload {
    Observation { property: String, value: String },
    Relation { predicate: String, to: StableId },
    Intent { action: String },
    Receipt { ok: bool, detail: String },
}

/// One consistent view of the graph: the store, its index, and the
/// snapshot identity every response carries.
pub struct Loaded {
    pub store: Store,
    pub index: Cortex,
    pub snapshot: Snapshot,
    /// Byte length of `events.jsonl` when this view was built.
    events_len: u64,
    /// Recorded activity, scanned at most once per graph version and
    /// shared by the timeline, entity histories and search.
    events: OnceLock<Vec<EventRow>>,
    /// The workspace's own judgments. These are pure functions of the
    /// graph, so they are computed once per version and shared — the
    /// alternative (recomputing per request, several times per request)
    /// cost six seconds a click.
    insights: OnceLock<Insights>,
    attention: OnceLock<Vec<Attention>>,
    findings: OnceLock<Vec<Finding>>,
    registry: OnceLock<BTreeMap<String, KindDef>>,
    fitness: OnceLock<Vec<TemplateFitness>>,
    /// The laid-out anatomy. Held for the same reason as the rest, and for
    /// one more: a layout that were recomputed per request would place
    /// things differently each time a tab reloaded.
    mri: OnceLock<crate::dto::MriView>,
    /// Every claim and its proof. Now reads it for the census, Evidence
    /// reads it whole; computing it twice per page would be waste.
    evidence: OnceLock<crate::dto::EvidenceView>,
    /// Which feature each thing serves, derived through the files a
    /// feature declares. Every list surface asks it per row.
    spine: OnceLock<brain_observe::spine::Spine>,
}

impl Loaded {
    /// The prefix this view is scoped to.
    pub fn prefix(&self) -> &str {
        &self.snapshot.prefix
    }

    /// Recorded activity in event-log order. The scan happens once per
    /// graph version; every later reader shares it.
    pub fn events(&self) -> &[EventRow] {
        self.events.get_or_init(|| scan_events(&self.store))
    }

    pub fn insights(&self) -> &Insights {
        self.insights.get_or_init(|| {
            twin::insights_with(&self.store, &self.index, self.prefix()).unwrap_or_default()
        })
    }

    pub fn attention(&self) -> &[Attention] {
        self.attention.get_or_init(|| {
            attention::attend_with(&self.store, &self.index, self.prefix(), self.insights())
                .unwrap_or_default()
        })
    }

    pub fn findings(&self) -> &[Finding] {
        self.findings.get_or_init(|| {
            coherence::check(&self.store, &self.index, self.prefix()).unwrap_or_default()
        })
    }

    /// Which feature everything serves. A pure function of the graph, so
    /// it is built once per version like every other judgment here.
    pub fn spine(&self) -> &brain_observe::spine::Spine {
        self.spine.get_or_init(|| {
            brain_observe::spine::build(&self.store, &self.index, self.prefix()).unwrap_or_default()
        })
    }

    pub fn registry(&self) -> &BTreeMap<String, KindDef> {
        self.registry
            .get_or_init(|| kinds::registry(&self.store, &self.index).unwrap_or_default())
    }

    /// What the brain learned about how well each contract works.
    pub fn fitness(&self) -> &[TemplateFitness] {
        self.fitness.get_or_init(|| {
            fitness::fitness(&self.store, &self.index, self.prefix(), None).unwrap_or_default()
        })
    }

    /// Every claim in the graph with its proof, computed once per version.
    pub fn evidence(&self) -> Result<crate::dto::EvidenceView, String> {
        if let Some(view) = self.evidence.get() {
            return Ok(view.clone());
        }
        let view = crate::query::evidence::build(self)?;
        Ok(self.evidence.get_or_init(|| view).clone())
    }

    /// The laid-out graph, computed once per version.
    pub fn mri(&self) -> Result<crate::dto::MriView, String> {
        if let Some(view) = self.mri.get() {
            return Ok(view.clone());
        }
        let view = crate::query::mri::build(self)?;
        Ok(self.mri.get_or_init(|| view).clone())
    }

    /// Compute the costly views up front so the first person to look does
    /// not wait for them.
    pub fn warm(&self) {
        self.events();
        self.insights();
        self.attention();
        // Before findings: the coherence pass reads the spine, and a
        // second build per version would be pure waste.
        self.spine();
        self.findings();
        self.registry();
        self.fitness();
        let _ = self.mri();
        let _ = self.evidence();
    }
}

fn scan_events(store: &Store) -> Vec<EventRow> {
    let Ok(history) = store.put_history() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(history.len());
    for id in history {
        let Ok(object) = store.get(&id) else { continue };
        let row = match object {
            Object::Observation {
                subject,
                property,
                value,
                source,
                observed_at_ms,
            } => EventRow {
                at_ms: observed_at_ms,
                subject: Some(subject),
                source,
                payload: EventPayload::Observation { property, value },
            },
            Object::Relation {
                from,
                predicate,
                to,
                source,
                observed_at_ms,
            } => EventRow {
                at_ms: observed_at_ms,
                subject: Some(from),
                source,
                payload: EventPayload::Relation { predicate, to },
            },
            Object::Intent { action, at_ms, .. } => EventRow {
                at_ms,
                subject: None,
                source: "governed change".to_string(),
                payload: EventPayload::Intent { action },
            },
            Object::Receipt {
                ok, detail, at_ms, ..
            } => EventRow {
                at_ms,
                subject: None,
                source: "governed change".to_string(),
                payload: EventPayload::Receipt { ok, detail },
            },
            _ => continue,
        };
        out.push(row);
    }
    out
}

pub struct AppState {
    pub config: Config,
    loaded: RwLock<Loaded>,
}

impl AppState {
    pub fn new(config: Config) -> Result<AppState, String> {
        let loaded = build(&config)?;
        loaded.warm();
        Ok(AppState {
            config,
            loaded: RwLock::new(loaded),
        })
    }

    /// Run a query against a current view of the graph. Rebuilds the index
    /// first when the event log grew; otherwise this is a `stat` and a read
    /// lock, so concurrent requests do not serialize behind each other.
    pub fn read<T>(&self, f: impl FnOnce(&Loaded) -> Result<T, String>) -> Result<T, String> {
        if self.is_stale()? {
            self.refresh()?;
        }
        let guard = self
            .loaded
            .read()
            .map_err(|_| "graph view is unavailable".to_string())?;
        f(&guard)
    }

    /// The current snapshot identity, refreshing first when the log grew.
    pub fn snapshot(&self) -> Result<Snapshot, String> {
        self.read(|loaded| Ok(loaded.snapshot.clone()))
    }

    fn is_stale(&self) -> Result<bool, String> {
        let current = events_len(&self.config.store_root);
        let guard = self
            .loaded
            .read()
            .map_err(|_| "graph view is unavailable".to_string())?;
        Ok(guard.events_len != current)
    }

    fn refresh(&self) -> Result<(), String> {
        let rebuilt = build(&self.config)?;
        let mut guard = self
            .loaded
            .write()
            .map_err(|_| "graph view is unavailable".to_string())?;
        *guard = rebuilt;
        Ok(())
    }
}

fn build(config: &Config) -> Result<Loaded, String> {
    let store = Store::open(&config.store_root).map_err(|e| e.to_string())?;
    let index = Cortex::open(&store).map_err(|e| e.to_string())?;
    let events_len = events_len(&config.store_root);
    let snapshot = Snapshot {
        prefix: config.prefix.clone(),
        head: store
            .head()
            .map_err(|e| e.to_string())?
            .map(|id| id.to_string()),
        cursor: store.put_history().map_err(|e| e.to_string())?.len(),
        objects: store.count_objects().map_err(|e| e.to_string())?,
        changed_at_ms: events_mtime_ms(&config.store_root),
        generated_at_ms: now_ms(),
    };
    Ok(Loaded {
        store,
        index,
        snapshot,
        events_len,
        events: OnceLock::new(),
        insights: OnceLock::new(),
        attention: OnceLock::new(),
        findings: OnceLock::new(),
        registry: OnceLock::new(),
        fitness: OnceLock::new(),
        mri: OnceLock::new(),
        evidence: OnceLock::new(),
        spine: OnceLock::new(),
    })
}

fn events_path(root: &std::path::Path) -> PathBuf {
    root.join("events.jsonl")
}

fn events_len(root: &std::path::Path) -> u64 {
    std::fs::metadata(events_path(root))
        .map(|meta| meta.len())
        .unwrap_or(0)
}

fn events_mtime_ms(root: &std::path::Path) -> u64 {
    std::fs::metadata(events_path(root))
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|delta| delta.as_millis() as u64)
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0)
        })
}
