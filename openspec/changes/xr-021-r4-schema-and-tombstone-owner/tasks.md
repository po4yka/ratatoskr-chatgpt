## 0. Plan

- [x] 0.1 Open this change (documentation; it cannot start from a failing test). Cites XR-021 CONTRACTS.md R4-04 and R4-10.

## 1. One schema definition (R4-10)

- [ ] 1.1 RED: `tests/persistence_schema.rs` `the_schema_adds_no_column_after_creating_a_table` and `applying_the_schema_twice_emits_no_already_exists_notice` fail on the seven `ALTER TABLE` statements.
- [ ] 1.2 GREEN: delete the seven statements and the comment above the second block from `schema.sql`.

## 2. The tombstone owner comes from the announced facts (R4-04)

- [ ] 2.1 RED: `tests/privacy_deletion_tombstones.rs` cases for a changed mapping, a mapping naming another user, an account that announced nothing, a mapping fallback, an announced account with no resolvable owner, and survivors re-announced under the announced owner, in archive, conversation, project and tenant scopes.
- [ ] 2.2 GREEN: `owner_for_deletion` replaces `owner_of_account` in the removal transaction; `downstream_tombstone_unbound` only for an announced account with no owner.

## 3. Documentation

- [ ] 3.1 `README.md` and `deploy/systemd/chatgpt.conf.example` describe the rule (documentation; no failing test).

## 4. Gate

- [ ] 4.1 `cargo fmt --all -- --check`, clippy with `-D warnings`, the 850-line check, `cargo test --workspace --locked`, `cargo deny --locked check`, `openspec validate --all --strict` and `openspec validate --archived`.
