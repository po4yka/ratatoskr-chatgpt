## Why

Ratatoskr can preserve and export ChatGPT archive evidence, but it cannot yet honor scoped privacy erasure or safely reprocess retained evidence when parser support improves. Plan item 9 closes that lifecycle gap and establishes the owner-authorized evidence process required before claiming support for a real provider export schema.

## What Changes

- Add tenant-authorized deletion by export archive, conversation, or tenant, with a complete preflight inventory covering raw and extracted blobs, normalized records, and downstream subjects.
- Execute deletion as a durable, replay-safe workflow: blob erasure is reference-aware, database removal and Knowledge tombstone outbox records commit with immutable audit evidence, and partial failures remain diagnosable and resumable.
- Add a `reparse` operator command that selects an explicitly newer compatible parser over preserved raw archives, supports a side-effect-free dry run with the same selection and comparison logic as apply mode, and converges without duplicate normalized revisions or events.
- Add parser-version migration planning and execution reports that enumerate eligible, changed, unchanged, unsupported, failed, and applied archives without introducing database migrations or a second API/contract version.
- Document a private owner-provided fixture discovery workflow that keeps personal exports out of Git, records consent and provenance privately, minimizes and redacts derived cases, and admits reviewed deterministic golden fixtures only after leak and hostile-input checks.
- Add test-first coverage for deletion completeness enumeration, deletion atomicity and replay, reparse idempotence and dry-run fidelity, migration report correctness, and golden-fixture admission.

## Capabilities

### New Capabilities

- `privacy-deletion`: Tenant-scoped deletion planning, execution, audit, downstream tombstones, retry, and completion guarantees.
- `archive-reparse`: Explicit-parser replay of preserved raw evidence, including dry-run fidelity and idempotent application.
- `parser-version-migration`: Fleet-style planning and reporting for moving eligible archives between declared parser versions without database migration tooling.
- `owner-fixture-discovery`: Private acquisition, minimization, review, and golden-test admission for owner-authorized real export fixtures.

### Modified Capabilities

- `archive-schema`: Add the current-schema records and constraints needed for privacy deletion jobs, reparse runs, reports, and audit, editing `schema.sql` in place.
- `blob-storage`: Add reference-aware, idempotent erasure of archive-owned blobs after database-backed reachability proves no retained evidence still references them.
- `parser-registry`: Expose deterministic compatible-version discovery and exact parser lookup for reparse and migration while retaining rejection of ambiguous automatic selection.

## Impact

- Affects the archive domain library, PostgreSQL schema, BlobStore boundary, parser registry, transactional outbox, service command parsing/execution, telemetry, integration tests, README/operator documentation, and fixture policy.
- Participates in changeset `AIARCH-009`: contracts add `user_requested` to the existing `ai_archive.subject.tombstoned.v1` reason vocabulary, Knowledge proves compatible deletion, and only then does this producer advance its pin and emit the reason.
- Affects every supported acquisition mode whose raw evidence is retained; the first implementation remains limited to parser/schema versions actually registered in this repository and does not claim real ChatGPT export support until the owner-fixture process succeeds.
- Adds no production dependency, no database migration file or migration tool, no API major version, and no browser/session automation.
