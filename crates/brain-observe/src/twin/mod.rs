//! The twin engine: continuous, drift-aware reflection of external software.
//!
//! `refresh` is the sense organ: it walks a source tree, compares reality
//! against the twin's latest claims, and records only what changed — new and
//! changed files get fresh observations, structure (symbols via
//! [`crate::symbols`]) and relations; vanished files get `present=false`;
//! unchanged files write nothing, so repeated refreshes do not grow the
//! graph. `status` runs the identical comparison read-only.
//!
//! Nothing is ever overwritten: the twin's history of a file is its
//! observation timeline, and "deleted" is itself just an observation.


mod deliverables;
pub use deliverables::*;
mod imports;
pub(crate) use imports::*;
mod insights;
pub use insights::*;
mod notes;
pub use notes::*;
mod reads;
pub use reads::*;
mod refresh;
pub use refresh::*;

#[cfg(test)]
mod tests;
