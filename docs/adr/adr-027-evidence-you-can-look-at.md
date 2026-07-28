# Evidence you can look at

Status: accepted

## Context

The brain could say a test failed. It could not show you the failure.

Three gaps, each small on its own:

- **Test reports were read for verdicts only.** `parse_junit` extracted a
  name and a pass/fail, and discarded the duration, the failure message,
  and — for Playwright — the `<attachment>` elements naming the screenshot,
  the video and the trace. The one report format that carries the evidence
  a person actually wants was parsed as if it were the one that does not.
- **Generated media was anonymous.** `assets::add` existed to give a
  screenshot a subtype, an owner and a staleness story, and had exactly one
  caller: the CLI. The documentation pipeline wrote six screenshots and two
  screencasts per run and declared none of them, so `assets::stale` could
  never fire for the artifacts it was written for.
- **Media could not be played.** Eyes had no HTTP Range support. The
  frontend already rendered `<video controls>`; Chrome could not seek it
  and Safari would not start it.

And one older gap that these surfaced: the graph records no command line
for any effect, yet a person looking at an action wants one.

## Decision

**Evidence is something you look at, and everything shown says whether it
was recorded or worked out.**

1. **Parse Playwright's JSON reporter directly.** It is the only report
   that names the spec file, keeps the error message, counts retries, and
   lists attachments. Each attachment becomes a declared asset
   `attached_to` the case that produced it, so a failing browser test in
   Eyes shows its screenshot beside its name. Attachment paths are resolved
   against the workspace and dropped if they land outside it or do not
   exist — the graph never names bytes it cannot point at.

2. **Run membership is recorded where it carries information.** A run
   links to the cases that failed, were skipped, or changed their mind
   (`failed`, `skipped`, `includes`). Passing cases are deliberately *not*
   linked to every run: that would add one edge per case per import — over
   a hundred per commit here — for a fact the case's own guarded `result`
   timeline already states. The Tests surface says what a run named; it
   does not imply the list is the run's full contents.

3. **Generated media declares itself.** `record_projection` now calls
   `assets::declare` for every image, screencast and audio file it writes,
   and stamps `rendered_from` on all of them rather than only the
   screenshots. They are given no `depicts` targets on purpose: a tour
   screenshot summarises a whole query, so listing the dozens of files
   behind it would mark it stale on every unrelated edit.

4. **The tour's freshness is a content claim, not a timestamp.** Because
   the narration is *computed* from graph queries, Eyes recomputes it and
   compares it sentence by sentence with the recorded `narration.txt`.
   When they differ it names the sentence that stopped being true. This is
   the artifact-rot problem stated precisely enough to show a person, and
   it is why `narrate` moved from the CLI into `brain_observe::tour`,
   where both the generator and the reader share one definition of what
   the tour is.

5. **Serve bytes properly.** Range requests answer `206` with
   `Content-Range` and `Accept-Ranges`; an unsatisfiable range is a `416`,
   not a whole file pretending to be a slice. Responses declare
   `Content-Length` rather than falling to chunked transfer — chunked cost
   the browser a flat twenty seconds per large response while `curl` saw
   two milliseconds.

6. **Recorded and reconstructed are different words.** An action's audit
   shows what the graph holds — the reason, the before and after hashes,
   the capability, the intent, the receipt, the verifying run — and marks
   as *reconstructed* anything Eyes worked out, such as the CLI command
   equivalent to a change. The graph records no invocation and no actor for
   historical intents; presenting a plausible one as a record is the exact
   failure this system exists to prevent.

## Consequences

- `brain testrun import` gains a `--dir` so attachment paths can be
  resolved; the default is the working directory.
- Attachments live wherever the reporter put them, usually `test-results/`.
  They are ingestible already (`png` and `webm` are in the twin's
  extension list), so a refresh after an import hashes them and they gain
  the ordinary freshness machinery.
- A failing feature claim can now disagree with its own definition of done:
  the DoD counts linked records, and Evidence resolves each linked record
  to its current state. "Tested — 3 records linked" and "that test is
  failing" are both true, and the disagreement is the point.
- The Playwright parser depends on a format Playwright controls. It reads
  defensively — unknown fields are ignored, a missing `results` array skips
  the case — and JUnit remains supported for every framework including
  Playwright.
