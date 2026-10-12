## Why

Round 4 of changeset XR-021 re-verified two defects against `origin/main` (CONTRACTS.md R4-04 and R4-10).

`schema.sql` carries seven migration-style `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` statements after `CREATE TABLE` statements that already declare the same columns. That is migration code in the one current schema definition, against the development status, and every start logs 42 `column ... already exists, skipping` notices (14 distinct statements, repeated).

A privacy deletion sends its tombstones to the owner the live `RATATOSKR__RECEIPT__PLATFORM_ACCOUNTS` mapping gives at deletion time, not to the owner the archive's own announced facts named. When the operator edits the mapping after an import, no tombstone is queued, every item is marked `downstream_tombstone_unbound`, the deletion completes, and Knowledge keeps the derived conversations, projects and search documents for good.

## What Changes

- `schema.sql` loses the seven `ALTER TABLE ... ADD COLUMN` statements and the comment above the second block. Every `CREATE TABLE IF NOT EXISTS` and index stays.
- The row-removal transaction of a deletion resolves the tombstone owner from the announced facts in `chatgpt_archive.outbox_events`, read before the matching rows are removed: the latest `ai_archive.archive.imported.v1` envelope of a deleted archive, else the first announced fact of a deleted conversation or project, else the latest imported envelope of the account, else any announced fact of the account. The live mapping is consulted only when no announced fact gives an owner.
- An account that announced no fact and has no mapping has nothing to retract: no tombstone, no unbound item. An account that announced facts but whose facts name no owner and has no mapping still reports `downstream_tombstone_unbound`.
- Survivors are re-announced under the same owner.
- `README.md` and `deploy/systemd/chatgpt.conf.example` describe the new rule.

Breaking, with no shims: a development database is recreated from the one definition, which already declares every column; and the tombstone owner no longer follows the live mapping once the archive announced a fact.

## Impact

Touches `schema.sql`, `crates/chatgpt-archive/src/privacy_deletion/` (new owner resolution), and the deletion tests. No wire shape changes. Cites XR-021 CONTRACTS.md R4-04 and R4-10. Out of scope: the survivor-content restore, the receipt, pump and capabilities code of Round 3, and the account-erasure transport.
