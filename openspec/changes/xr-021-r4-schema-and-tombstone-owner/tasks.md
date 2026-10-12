## 0. Plan

- [x] 0.1 Open this change (documentation; it cannot start from a failing test). Cites XR-021 CONTRACTS.md R4-04 and R4-10.

## 1. One schema definition (R4-10)

- [x] 1.1 RED: `tests/persistence_schema.rs` `the_schema_adds_no_column_after_creating_a_table` failed listing the seven `ALTER TABLE` statements (lines 75, 312, 314, 316, 318, 343, 345), and `applying_the_schema_twice_emits_no_already_exists_notice` failed with 14 `column ... already exists, skipping` notices.
- [x] 1.2 GREEN: the seven statements and the comment above the second block are deleted from `schema.sql`; both tests pass.

## 2. The tombstone owner comes from the announced facts (R4-04)

- [x] 2.1 RED: `tests/privacy_deletion_tombstones.rs` `a_deletion_tombstones_to_the_owner_the_archive_was_announced_under_when_the_mapping_changed` (archive scope: "a tombstone must be queued"), `a_deletion_uses_the_announced_owner_even_when_the_mapping_names_another_user` (the tombstone named the mapped user instead of the announced one), `an_account_that_announced_nothing_completes_without_a_tombstone_or_an_unbound_item` (the report claimed `downstream_tombstone_unbound = 3`) and `survivors_are_re_announced_under_the_announced_owner` (an unbound item and no re-announcement) failed; the mapping fallback and the announced-without-owner cases stayed green as regression guards. Archive, conversation and tenant scopes each run the changed-mapping cases.
- [x] 2.2 GREEN: `privacy_deletion/owner.rs` `owner_for_deletion` replaces `owner_of_account` in the removal transaction; `downstream_tombstone_unbound` only for an announced account with no owner; all eight tests of the file pass.

## 3. Documentation

- [ ] 3.1 `README.md` and `deploy/systemd/chatgpt.conf.example` describe the rule (documentation; no failing test).

## 4. Gate

- [ ] 4.1 `cargo fmt --all -- --check`, clippy with `-D warnings`, the 850-line check, `cargo test --workspace --locked`, `cargo deny --locked check`, `openspec validate --all --strict` and `openspec validate --archived`.
