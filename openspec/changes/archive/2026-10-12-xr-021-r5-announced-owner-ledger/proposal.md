## Why

XR-021 CONTRACTS.md R5-06 (replacing R4-04's mechanism). `privacy_deletion/owner.rs::owner_for_deletion` scrapes the owner out of `outbox_events` envelopes with `LIMIT 1` for the whole request and applies that one owner to every tombstone. A tenant or multi-archive deletion that spans archives announced under different Platform users therefore addresses the older users' tombstones to the wrong tenant, which Knowledge Terms as `foreign_subject` and keeps the data. The scrape also depends on outbox rows that a deletion removes, and it falls back to the live `PLATFORM_ACCOUNTS` mapping.

## What Changes

- `schema.sql` gains `chatgpt_archive.announced_subjects`, a ledger with one row per `(subject_kind, subject_id, owner_ref)`. It has no foreign key to `exports`, so deleting an export never cascades it.
- `enqueue_normalized_event` records the ledger row in the same transaction that enqueues each `ai_archive.archive.imported`, `conversation.added|updated` and `project.added|updated` fact, with the owner the fact carries. The facts a deletion re-announces go through the same call.
- A deletion emits one tombstone per distinct owner the ledger holds for each tombstone subject, with the deduplication key `privacy-delete:<request>:<subject>:<owner>`, then deletes that subject's ledger rows. A subject with no ledger row emits nothing, adds no `downstream_tombstone_unbound` gap and does not fail the request.
- A re-announced survivor takes the owner of the retained archive's ledger row.
- `privacy_deletion/owner.rs`, `FACT_OWNER` and `PrivacyDeletionService::with_platform_users` are deleted.

Breaking, with no shims: a deletion never reads the live mapping, `PrivacyDeletionService::with_platform_users` no longer exists (the operator `privacy-delete` command stops passing it), and `downstream_tombstone_unbound` is no longer produced.

## Impact

Touches `schema.sql`, `crates/chatgpt-archive/src/{outbox.rs, contract_projection/reannounce.rs, privacy_deletion/*}`, `services/chatgpt-archive/src/lifecycle_commands.rs` and the deletion tests. Cites XR-021 CONTRACTS.md R5-06. Out of scope: the latent fallback for a conversation with no export observation and fleet-wide account erasure.
