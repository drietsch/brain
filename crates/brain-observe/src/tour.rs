//! The generated tour, defined once.
//!
//! `brain docs generate` produces a tour of the graph: a chapter per
//! query, a screenshot of each chapter's output, a narration script
//! computed from the same graph, and a screencast with that narration
//! spoken over it. Eyes then *shows* that tour — the video, the chapters,
//! the script.
//!
//! Both sides need the same answer to "what is a chapter, and what should
//! the narration say right now", so the definition lives here rather than
//! in the generator. That also buys the freshness check that matters:
//! because the narration is computed, Eyes can recompute it and compare
//! it against the recorded `narration.txt`. When they differ, the tour is
//! demonstrably out of date, and we can name the sentence that changed —
//! a content-level staleness claim rather than a timestamp guess.

use brain_store::{Store, StoreError};

/// One chapter of the tour: a command, its title, and the screenshot its
/// output produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chapter {
    pub id: &'static str,
    pub title: &'static str,
    /// Argument vector, without the binary name.
    pub args: Vec<String>,
    /// Path of the screenshot, relative to the generated docs directory.
    pub image: String,
    /// Which narration sentence speaks about this chapter, if any.
    ///
    /// The script is not one sentence per chapter — it is conditional, and
    /// it runs in its own order — so the two are matched by topic rather
    /// than by position. Guessing an alignment would put the wrong words
    /// under the wrong picture.
    pub topic: Option<&'static str>,
}

impl Chapter {
    /// The command as a person would type it.
    pub fn command(&self) -> String {
        format!("brain {}", self.args.join(" "))
    }
}

/// The chapters, in tour order.
pub fn chapters(prefix: &str) -> Vec<Chapter> {
    let spec: &[(&'static str, &'static str, &[&str], Option<&'static str>)] = &[
        (
            "insights",
            "Insights (`brain twin insights`)",
            &["twin", "insights"],
            Some("size"),
        ),
        (
            "matrix",
            "Feature matrix — definition of done (`brain feature matrix`)",
            &["feature", "matrix"],
            Some("features"),
        ),
        (
            "decisions",
            "Decisions (`brain adr list`)",
            &["adr", "list"],
            Some("decisions"),
        ),
        (
            "tests",
            "Tests (`brain twin tests`)",
            &["twin", "tests"],
            Some("tests"),
        ),
        (
            "protocols",
            "Test protocols (`brain testrun list`)",
            &["testrun", "list"],
            Some("tests"),
        ),
        ("attention", "Attention (`brain attend`)", &["attend"], None),
    ];
    spec.iter()
        .map(|(id, title, args, topic)| Chapter {
            id,
            title,
            args: args
                .iter()
                .map(|a| a.to_string())
                .chain(std::iter::once(prefix.to_string()))
                .collect(),
            image: format!("img/{id}.png"),
            topic: *topic,
        })
        .collect()
}

/// Narration computed straight from the graph — the audio track is as
/// regenerated (and as trustworthy) as the text.
pub fn narrate(store: &Store, prefix: &str) -> Result<String, StoreError> {
    Ok(narration_lines(store, prefix)?.join("\n"))
}

/// The narration, one sentence per line.
pub fn narration_lines(store: &Store, prefix: &str) -> Result<Vec<String>, StoreError> {
    Ok(narration_lines_from(&crate::twin::insights(store, prefix)?, prefix))
}

/// The narration as sentences, each tagged with what it is about, so a
/// chapter can find the words that belong to it.
pub fn narration_lines_from(ins: &crate::twin::Insights, prefix: &str) -> Vec<String> {
    narration_from(ins, prefix)
        .into_iter()
        .map(|(_, sentence)| sentence)
        .collect()
}

/// The narration from insights the caller already has, tagged by topic.
///
/// A reader that shows the tour has usually just computed the same
/// picture; recomputing it here cost Eyes nearly two seconds a page.
pub fn narration_from(ins: &crate::twin::Insights, prefix: &str) -> Vec<(&'static str, String)> {
    let mut lines = vec![("intro", format!(
        "This is the live tour of {}, generated directly from the semantic graph.",
        prefix.replace('/', " ")
    ))];
    lines.push((
        "size",
        format!(
            "The twin currently tracks {} files, {} symbols, and {} relations.",
            ins.files, ins.symbols, ins.relations
        ),
    ));
    if ins.tests_declared > 0 {
        match ins.last_run {
            Some((_, total, passed, failed)) => {
                let verdict = if failed == 0 {
                    "all passing".to_string()
                } else {
                    format!("{failed} failing")
                };
                lines.push((
                    "tests",
                    format!(
                        "{} tests are declared; the last imported run had {passed} of {total} passing — {verdict}.",
                        ins.tests_declared
                    ),
                ));
            }
            None => lines.push((
                "tests",
                format!("{} tests are declared.", ins.tests_declared),
            )),
        }
    }
    if !ins.features.is_empty() {
        let done = ins.features.iter().filter(|f| f.done).count();
        lines.push((
            "features",
            format!(
                "The feature matrix shows {} registered features, {done} of them meeting the full definition of done.",
                ins.features.len()
            ),
        ));
    }
    if !ins.decisions.is_empty() {
        lines.push((
            "decisions",
            format!(
                "{} architecture decisions are active, each linked to the files it concerns.",
                ins.decisions.len()
            ),
        ));
    }
    let stale_warns = ins
        .stale_docs
        .iter()
        .filter(|d| d.severity == crate::twin::Severity::Warn)
        .count();
    if ins.stale_docs.is_empty() {
        lines.push((
            "staleness",
            "No documentation is stale: every doc is newer than the files it mentions.".to_string(),
        ));
    } else if stale_warns > 0 {
        lines.push((
            "staleness",
            format!("{stale_warns} document(s) have gone stale and need attention."),
        ));
    } else {
        lines.push((
            "staleness",
            format!(
                "{} record(s) quietly aged behind the code they describe.",
                ins.stale_docs.len()
            ),
        ));
    }
    lines.push((
        "outro",
        "Everything you just heard was a query, not prose — regenerate this tour any time with one command."
            .to_string(),
    ));
    lines
}

/// A sentence the tour still asserts that the graph no longer supports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drifted {
    /// What the recording says, or `None` when the tour never said it.
    pub recorded: Option<String>,
    /// What the graph would say now, or `None` when the sentence is gone.
    pub current: Option<String>,
}

/// Compare a recorded narration against what the graph would say now.
///
/// Empty means the tour is still true. Anything else is a claim on disk
/// that the graph has since contradicted — the artifact-rot problem,
/// stated precisely enough to show a person.
pub fn narration_drift(
    store: &Store,
    prefix: &str,
    recorded: &str,
) -> Result<Vec<Drifted>, StoreError> {
    Ok(narration_drift_from(
        &crate::twin::insights(store, prefix)?,
        prefix,
        recorded,
    ))
}

/// `narration_drift` against insights the caller already holds.
pub fn narration_drift_from(
    insights: &crate::twin::Insights,
    prefix: &str,
    recorded: &str,
) -> Vec<Drifted> {
    let current = narration_lines_from(insights, prefix);
    let recorded: Vec<String> = recorded
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();

    // Sentences are positional and stable in shape, so a line-by-line
    // comparison names the fact that changed rather than reporting the
    // whole script as different.
    let mut out = Vec::new();
    for index in 0..current.len().max(recorded.len()) {
        let was = recorded.get(index);
        let now = current.get(index);
        if was.map(String::as_str) == now.map(String::as_str) {
            continue;
        }
        out.push(Drifted {
            recorded: was.cloned(),
            current: now.cloned(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narration_is_computed_from_graph_state_and_knows_when_it_aged() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "pub fn main() {}\n").unwrap();
        let store = Store::open(&dir.path().join(".brain")).unwrap();
        crate::twin::refresh(&store, dir.path(), "twin/app").unwrap();

        let script = narrate(&store, "twin/app").unwrap();
        assert!(script.contains("twin app"), "the prefix is spoken, not typed");
        assert!(script.contains("1 files"));
        assert!(narration_drift(&store, "twin/app", &script).unwrap().is_empty());

        // The graph moves; the recording does not.
        std::fs::write(dir.path().join("src/other.rs"), "pub fn other() {}\n").unwrap();
        crate::twin::refresh(&store, dir.path(), "twin/app").unwrap();
        let drift = narration_drift(&store, "twin/app", &script).unwrap();
        assert_eq!(drift.len(), 1, "exactly one sentence stopped being true");
        assert!(drift[0].recorded.as_ref().unwrap().contains("1 files"));
        assert!(drift[0].current.as_ref().unwrap().contains("2 files"));
    }

    #[test]
    fn every_chapter_names_its_command_and_its_screenshot() {
        let chapters = chapters("twin/self");
        assert_eq!(chapters.len(), 6);
        let tests = chapters.iter().find(|c| c.id == "tests").unwrap();
        assert_eq!(tests.command(), "brain twin tests twin/self");
        assert_eq!(tests.image, "img/tests.png");
    }
}
