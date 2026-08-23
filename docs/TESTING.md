# ChatGPT archive testing strategy

Maintain synthetic fixtures for multiple observed schema versions and cases: no projects, projects/instructions, branches/regeneration, files/images/Canvas, citations/tools, unknown records, missing/orphan relations, duplicates, malformed/large archives, and partial assets.

Required tests:

- Streaming hash/idempotent duplicate intake and crash recovery.
- Archive traversal/count/size/decompression/MIME limits.
- Schema detection/parser selection and forward-compatible unknown preservation.
- Conversation graph, revisions, project/source/asset reconciliation.
- Completeness counts/status/warnings and missing-data-not-deletion semantics.
- Portable export deterministic manifest and safe paths.
- Privacy deletion, authorization, schema initialization, outbox/inbox replay, redacted telemetry.
- Optional Compliance cursor/redelivery/auth tests with fakes.
- Planned workspace Export Agent -> ChatGPT -> Knowledge flow.

Real personal exports are never committed; sanitized fixtures require explicit review.

## Test-first

A change is planned before it is built, and the plan is a task list in which behaviour arrives in
pairs: one task adds a failing test, the next makes it pass. `openspec/config.yaml` carries that
rule, which is what puts it into every planning and implementation request rather than only into this
document.

The loop:

1. Write the test the scenario names. Run it. Confirm it fails, and read the failure — a test that
   fails because it does not compile has proved nothing about the behaviour.
2. Write the smallest change that makes it pass. Run it again.
3. Refactor only once it is green, adding no test and changing no behaviour.

Two checks stand behind this, and neither of them can see the order:

- `openspec validate --archived`, in `.github/workflows/openspec.yml`, fails when a change was
  archived with a task left unticked.
- A step in `.github/workflows/fleet.yml` fails when this repository holds a manifest and a `ci.yml`
  that never runs a test.

`ratatoskr-workspace/docs/QUALITY_GATES.md` records why the order itself is not checkable, rather
than leaving the gap to be discovered.
