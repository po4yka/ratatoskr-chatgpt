## Why

Changeset XR-021 fixes cross-repository integration defects, and four of them land in the ChatGPT archive service. The binding text is XR-021 CONTRACTS.md sections S05, S06 (D1 to D3) and S07; this proposal cites them and does not restate them.

- Platform's finalize dead-ends on the receipt listener: there is no capability document, and a wrong method answers with an empty body that Edge replaces with `edge.upstream_invalid_response` (S06 D1, D2).
- A partially imported archive reports `partially_succeeded` with neither a warning nor an error, which the contracts now reject (S06 D3).
- No `ai_archive.*` fact is ever built. Only tombstones are enqueued, as bare payloads with an archive-internal owner that matches no Knowledge tenant, and the publisher selects only the operation report (S07).
- Privacy deletion fails deterministically on `platform_operation_imports` and `reparse_runs` foreign keys after the raw blob is already erased, strands rows that point at a deleted first-seen export, and erases bytes before the rows that reference them are gone (S07).
- The operator listener default 9084 collides with Threads and Claude (S05).

## What Changes

- Pin all five `ratatoskr-*` contract dependencies to the XR-021 contracts commit.
- Serve `GET /v1/capabilities` on the receipt router and answer every non-2xx of the receipt routes with an error envelope, including 405. Import the `platform_receipt` constants instead of local copies.
- Report an incomplete import with exactly the contract warning.
- Store complete `EventEnvelope` documents in the outbox, publish all archive event types through one ack-gated pump over a closed subject mapping, and constrain `event_type` in the schema.
- Persist the projects and conversation detail the facts need, then enqueue `archive.imported`, `project.added|updated` and `conversation.added|updated` in the transaction that persists an initial import or an applied reparse. The owner is the Platform user resolved from the configured account mapping.
- Rework privacy deletion into rows first, bytes second, complete last: delete the dependent `platform_operation_imports` and `reparse_runs`, repoint retained provenance, recompute blob sharing against remaining rows, emit contract-conformant tombstones with the Platform owner, and keep unpublished tombstones of earlier requests.
- Move the operator listener default to 9085.

Breaking, with no shims: `FinalizationFault` gains `AfterRowsCommitted`; `PrivacyDeletionService` gains `with_platform_users`; `schema.sql` changes in place (outbox `event_type` check, deletion request `evidence_ref` and `rows_removed_at`, completeness report `gaps`); the operator port moves from 9084 to 9085; `NormalizedArchiveEvent` carries an envelope, not a bare payload.

## Impact

Touches receipt HTTP, the report builder, the outbox and its pump, the import and reparse persistence paths, privacy deletion, `schema.sql`, configuration defaults and documentation. Platform-side changes, ACL lines, Knowledge consumption, closing the cross-tenant content-addressed blob race, account erasure transport and a general account registry are out of scope.
