# Proposal: authenticated-archive-receipt

## Why

Implementation plan item 2 is the first vertical slice above the scaffold: without an authenticated receipt surface that streams, hashes, caps, and durably stores export archives, nothing downstream (inspection, parsing, reconciliation) has raw evidence to work on. The scaffold's error taxonomy already reserves the public routing kinds for "the first public routes arrive with implementation plan item 2", and `schema.sql` carries planned `exports`/`import_runs` relations that no code populates yet.

## What Changes

- New authenticated, tenant-scoped HTTP receipt surface: `POST /exports` accepts a streamed export archive with a bearer credential that resolves to one archive account, an explicit acquisition mode header, and a media type.
- Receipt streams the body through SHA-256 while teeing bytes to an isolated staging file; no full-file buffering in memory at any point.
- Declared (`Content-Length`) and received byte totals are enforced against a configurable maximum archive size before anything is published.
- Received bytes are published through the existing content-addressed `BlobStore` (write-once, hard-link publish), then recorded as an immutable raw export row with digest, byte length, acquisition mode, and blob reference.
- A durable import state machine is introduced: `received -> hashed -> stored -> inspected -> parsed -> reconciled -> completed | partial`, with `failed` reachable from any non-terminal stage and `quarantined` reserved. Transitions are guarded compare-and-set updates so replayed or out-of-order commands cannot regress terminal state, and interrupted runs resume from their persisted stage when their staging evidence survives.
- Duplicate-archive detection by `(account, SHA-256)` digest: re-receiving identical content answers with an explicit duplicate outcome naming the existing export and writes no new rows; different content under the same tenant is stored as a new export.
- The first-version schema definition is edited in place (development status: no migrations): exports become strictly tenant-scoped with per-account digest uniqueness; import runs gain the columns the state machine needs (nullable export link until the export row materializes, digest/length captured at the hashed stage).
- New declared configuration keys follow the closed-key contract: maximum archive size, receipt staging root, and tenant bearer tokens.

Out of scope (later plan items): archive inspection/extraction and parser registry (item 3), conversation parsing (item 4), reconciliation and completeness (item 5), event publication, portable export, Compliance adapters. No parsing of archive contents happens in this change; receipt stores bytes as opaque evidence.

## Capabilities

### New Capabilities

- `archive-receipt`: Authenticated, tenant-scoped receipt of export archives — streaming hashing, size caps, immutable raw storage via BlobStore, hostile-input rejection (truncation, oversize), and duplicate-archive outcomes by digest.
- `import-state`: The durable import state machine — declared states, legal transitions, guarded non-regressing updates, terminal protection, idempotent re-entry, and crash recovery from persisted stages plus surviving staging evidence.

### Modified Capabilities

- `runtime-configuration`: Adds three declared keys to the closed key set (`RATATOSKR__LIMITS__MAX_ARCHIVE_BYTES`, `RATATOSKR__STORAGE__RECEIPT_STAGING_ROOT`, `RATATOSKR__RECEIPT__TENANT_TOKENS`), keeping every-violations reporting and secret redaction intact.
- `archive-schema`: Reshapes `chatgpt_archive.exports` (tenant-scoped uniqueness, mandatory owning account) and `chatgpt_archive.import_runs` (state set owned by the import-state capability, nullable export link, digest/byte-length captured per run). Still one embedded definition file applied transactionally, edited in place.

## Impact

- `crates/chatgpt-archive`: new `receipt` module tree (auth, state machine, receiver, repository seam, HTTP router); extensions to `config.rs`, `error.rs` (new public failure kind), `lib.rs` exports.
- `services/chatgpt-archive`: wires the receipt router beside the admin plane when staging is configured; best-effort startup sweep resumes interrupted runs.
- `schema.sql`: edited in place as described above; `tests/persistence_schema.rs` remains the integration proof.
- Dependencies: none added. Uses existing axum/http-body-util streaming, sha2, sqlx, ratatoskr-identifiers.
- Cross-repository: none. The upload API is not yet consumed by Platform/export-agent; the fleet `blob-references` store spec is cited, not changed.
