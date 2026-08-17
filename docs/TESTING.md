# ChatGPT archive testing strategy

Maintain synthetic fixtures for multiple observed schema versions and cases: no projects, projects/instructions, branches/regeneration, files/images/Canvas, citations/tools, unknown records, missing/orphan relations, duplicates, malformed/large archives, and partial assets.

Required tests:

- Streaming hash/idempotent duplicate intake and crash recovery.
- Archive traversal/count/size/decompression/MIME limits.
- Schema detection/parser selection and forward-compatible unknown preservation.
- Conversation graph, revisions, project/source/asset reconciliation.
- Completeness counts/status/warnings and missing-data-not-deletion semantics.
- Portable export deterministic manifest and safe paths.
- Privacy deletion, authorization, migrations, outbox/inbox replay, redacted telemetry.
- Optional Compliance cursor/redelivery/auth tests with fakes.
- Workspace export-agent -> ChatGPT -> Knowledge flow.

Real personal exports are never committed; sanitized fixtures require explicit review.
