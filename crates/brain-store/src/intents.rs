//! The durable intent log: crash safety for consequential effects.
//!
//! Protocol (the ordering is the whole point):
//!
//! 1. `begin(intent)` durably records the intent BEFORE the effect is attempted.
//! 2. The effect runs.
//! 3. `confirm`/`fail` records the receipt.
//!
//! If the process dies between 1 and 3, `recover()` finds the pending intent
//! and marks it INDETERMINATE. An indeterminate effect is never automatically
//! retried — the external world may or may not have changed. Reconciliation
//! (checking external reality, idempotency, or asking for authority) is a
//! separate, deliberate act by the caller. `recover()` only ever changes
//! labels; it never re-executes anything.
//!
//! The log is log-structured: state of an intent = last line mentioning it.

use brain_core::ids::NodeId;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::PathBuf;

use crate::{now_ms, StoreError};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentState {
    /// Intent recorded, outcome not yet known. Normal mid-flight state;
    /// abnormal if found at recovery time.
    Pending,
    /// Effect completed and receipt recorded.
    Confirmed,
    /// Effect definitively failed and receipt recorded.
    Failed,
    /// A crash or loss of contact left the outcome unknown. Requires
    /// reconciliation; must never be blindly retried.
    Indeterminate,
}

#[derive(Debug, Serialize, Deserialize)]
struct LogLine {
    at_ms: u64,
    intent: String,
    state: IntentState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    receipt: Option<String>,
}

pub struct IntentLog {
    path: PathBuf,
}

impl IntentLog {
    pub fn new(path: PathBuf) -> Self {
        IntentLog { path }
    }

    fn append(&self, line: &LogLine) -> Result<(), StoreError> {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{}", serde_json::to_string(line)?)?;
        // Durability before the effect is attempted is the contract.
        f.sync_all()?;
        Ok(())
    }

    /// Record intent BEFORE attempting the effect.
    pub fn begin(&self, intent: NodeId) -> Result<(), StoreError> {
        self.append(&LogLine {
            at_ms: now_ms(),
            intent: intent.to_string(),
            state: IntentState::Pending,
            receipt: None,
        })
    }

    pub fn confirm(&self, intent: NodeId, receipt: NodeId) -> Result<(), StoreError> {
        self.append(&LogLine {
            at_ms: now_ms(),
            intent: intent.to_string(),
            state: IntentState::Confirmed,
            receipt: Some(receipt.to_string()),
        })
    }

    pub fn fail(&self, intent: NodeId, receipt: NodeId) -> Result<(), StoreError> {
        self.append(&LogLine {
            at_ms: now_ms(),
            intent: intent.to_string(),
            state: IntentState::Failed,
            receipt: Some(receipt.to_string()),
        })
    }

    /// Current state of every intent ever begun (intent id string -> state).
    pub fn states(&self) -> Result<BTreeMap<String, IntentState>, StoreError> {
        let mut out = BTreeMap::new();
        if !self.path.exists() {
            return Ok(out);
        }
        for line in fs::read_to_string(&self.path)?.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let parsed: LogLine = serde_json::from_str(line)?;
            out.insert(parsed.intent, parsed.state);
        }
        Ok(out)
    }

    /// Crash recovery: every intent still Pending is marked Indeterminate.
    /// Returns the affected intent ids. Marks only — never re-executes.
    pub fn recover(&self) -> Result<Vec<String>, StoreError> {
        let mut marked = Vec::new();
        for (intent, state) in self.states()? {
            if state == IntentState::Pending {
                self.append(&LogLine {
                    at_ms: now_ms(),
                    intent: intent.clone(),
                    state: IntentState::Indeterminate,
                    receipt: None,
                })?;
                marked.push(intent);
            }
        }
        Ok(marked)
    }

    /// Summary counts by state, for `brain status`.
    pub fn summary(&self) -> Result<serde_json::Value, StoreError> {
        let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
        for state in self.states()?.values() {
            let key = match state {
                IntentState::Pending => "pending",
                IntentState::Confirmed => "confirmed",
                IntentState::Failed => "failed",
                IntentState::Indeterminate => "indeterminate",
            };
            *counts.entry(key).or_default() += 1;
        }
        Ok(json!(counts))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;
    use brain_core::object::{hash_object, Object};

    fn intent_object(n: u64) -> Object {
        Object::Intent {
            action: "io/echo".to_string(),
            arg_hash: brain_core::canonical::hash_bytes(&n.to_le_bytes()),
            capability: Some("io".to_string()),
            at_ms: n,
        }
    }

    #[test]
    fn confirmed_intents_stay_confirmed() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let log = store.intents();

        let intent = store.put(&intent_object(1)).unwrap();
        let receipt = hash_object(&intent_object(2)).unwrap();
        log.begin(intent).unwrap();
        log.confirm(intent, receipt).unwrap();

        assert_eq!(
            log.states().unwrap().get(&intent.to_string()),
            Some(&IntentState::Confirmed)
        );
        assert!(log.recover().unwrap().is_empty());
    }

    #[test]
    fn crash_between_intent_and_receipt_becomes_indeterminate_not_retried() {
        let dir = tempfile::tempdir().unwrap();
        let intent;
        {
            // Simulated process #1: records intent, then "crashes" before
            // any receipt is written.
            let store = Store::open(dir.path()).unwrap();
            intent = store.put(&intent_object(3)).unwrap();
            store.intents().begin(intent).unwrap();
        }
        {
            // Simulated process #2: recovery pass over the same durable state.
            let store = Store::open(dir.path()).unwrap();
            let log = store.intents();
            let marked = log.recover().unwrap();
            assert_eq!(marked, vec![intent.to_string()]);
            assert_eq!(
                log.states().unwrap().get(&intent.to_string()),
                Some(&IntentState::Indeterminate)
            );
            // Recovery is idempotent and still does not retry.
            assert!(log.recover().unwrap().is_empty());
        }
    }
}
