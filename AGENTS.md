<!-- brain:begin instructions — generated from the kind registry by `brain instructions generate`; edit rules with `brain template set`, never here -->

## Brain guardrails

Orient with `brain wake twin/self` before working; consolidate with `brain sleep twin/self` before finishing.

Artifact kinds (where truth lives, how to author):

| kind | placement | lives at | author via | requires | enforcement |
|---|---|---|---|---|---|
| asset | file_first | docs/assets/** | write the file; the twin captures it |  | advisory |
| capability_matrix | projection | docs/brain/capability-matrix/{slug}.md | rendered query — never authored |  | advisory |
| decision | file_first | docs/adr/*.md | write the file; the twin captures it | title, status | advisory |
| doc | file_first | README.md, docs/*.md | write the file; the twin captures it | title | advisory |
| plan | graph_first | docs/brain/plans/{slug}.md | `brain artifact new twin/self plan <slug>` | title | advisory |
| prototype | file_first | prototypes/** | write the file; the twin captures it | title | advisory |
| runbook | file_first | docs/runbooks/** | write the file; the twin captures it | title | advisory |
| task_list | graph_first | docs/brain/task-lists/{slug}.md | `brain artifact new twin/self task_list <slug>` | title | advisory |

Rules:

- Files under `docs/brain/` are **read-only projections** of the graph. Edit through `brain artifact edit twin/self <kind> <slug>`, never the file.
- Finished plans: `brain plan done twin/self <slug>`. A doc reviewed and still accurate: `brain adr|plan|artifact ack` (resets its staleness clock).
- A wrong or outdated link: `brain relation retract <from> <predicate> <to>`.
- Binary assets (screenshots, HTML templates): `brain asset add <file> --prefix twin/self --for <kind>/<slug> --depicts <path>` — declared links are their staleness story.
- Enforced kinds refuse nonconforming writes (exit 3) and, with the pre-commit gate, block commits; the error names the fix.

<!-- brain:end instructions -->
