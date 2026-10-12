## 0. Plan

- [x] 0.1 Open this change (documentation; it cannot start from a failing test). Cites XR-021 CONTRACTS.md R5-06.

## 1. The ledger

- [x] 1.1 RED: `tests/announced_subjects.rs` (real PostgreSQL) fails on its assertions with a stub `announced_subjects::record` that inserts nothing: an import under mapping A leaves no ledger rows for the archive, the conversation and the project; a reparse that changes one digest adds nothing for an unchanged subject; an unmapped account writes no row; deleting an export does not remove the rows.
- [x] 1.2 GREEN: the table in `schema.sql`, `announced_subjects::record` called from `enqueue_normalized_event`; the tests pass.

## 2. One tombstone per owner

- [x] 2.1 RED: `tests/privacy_deletion_owner.rs` failed on four assertions: a tenant-scope deletion over two archives announced under two users queued only user B's tombstones (one owner for the whole request), a re-announced survivor carried user A instead of the retained archive's user B, the ledger rows of a tombstoned subject remained, and `privacy_deletion/owner.rs` still existed. The announced-owner case across the three scopes and the never-announced case passed already (through the outbox scraping) and stay as regression guards.
- [x] 2.2 GREEN: `privacy_deletion/owner.rs` is deleted, `insert_tombstones` reads the ledger, `reannounce` takes the retained archive's owner, `PrivacyDeletionService::with_platform_users` is removed and the existing deletion tests seed the ledger through a real import.

## 3. Documentation

- [x] 3.1 `README.md` describes the ledger and the per-owner tombstones (documentation; no failing test).

## 4. Gate

- [ ] 4.1 `cargo fmt --all -- --check`, clippy with `-D warnings`, the 850-line check, `cargo test --workspace --locked`, `cargo deny --locked check`, `openspec validate --all --strict` and `openspec validate --archived` pass locally.
