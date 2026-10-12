## Decisions

- The owner is recorded when a subject is announced, not reconstructed later. The ledger row is written by `enqueue_normalized_event` itself, from the typed event (kind, subject id, archive id, owner), in the transaction that inserts the outbox row, so no fact can be enqueued without its ledger row and a rolled-back import leaves neither.
- The ledger row is inserted only when the outbox insert added a row. A deduplicated repeat announces nothing new.
- The primary key is `(subject_kind, subject_id, owner_ref)`: a conversation announced under two Platform users has two rows and gets two tombstones. `ai_archive_id` is the archive the subject was announced under for that owner and becomes the `ai_archive_id` of the tombstone, so a conversation-scope deletion no longer needs an archive subject in the same request.
- `account_id` references `accounts` with `ON DELETE CASCADE`, so a tenant-scope deletion removes the account's remaining ledger rows with the account. There is no reference to `exports`.
- Tombstones are built from the ledger inside the row-removal transaction before the ledger rows of the subject are deleted; the rollback of an injected fault restores both.
- A survivor is announced again under the owner of the retained archive's most recent ledger row. A retained archive with no ledger row (never announced) has nothing to repair and is skipped.
- The live mapping stays a constructor parameter of `InitialImport` and `ReparseEngine` only, to choose the owner of a new fact.
