## 0. Plan

- [x] 0.1 Open this change (documentation; it cannot start from a failing test). Cites XR-021 CONTRACTS.md R5-12.

## 1. Sequence recording

- [ ] 1.1 RED: `tests/stream_sequence.rs` (real broker) fails on its assertions with a pump that stores no sequence: after publication `published_stream_seq` is null while the stored stream message has a sequence; a duplicate PubAck must still carry the original.
- [ ] 1.2 GREEN: `published_stream_seq` in `schema.sql`, `mark_published(id, sequence)` from the `PublishAck`; the tests pass.

## 2. The command

- [ ] 2.1 RED: `tests/stream_erasure_request.rs` (real PostgreSQL, then real broker) fails with a stub `drop_published_fact_rows` that returns no sequences: deleting an archive whose facts were published enqueues no command, a tenant deletion covers no fact, more than 500 sequences are not split, an unpublished fact must add none, tombstones and operation reports must never be listed, the pump has no subject for the command type.
- [ ] 2.2 GREEN: `drop_published_fact_rows`, `stream_erasure.rs`, the pump subject and the schema `CHECK`; the tests pass, including the fake Platform handler that deletes the named sequence and leaves another owner's message alone.

## 3. Documentation and ACL copy

- [ ] 3.1 `README.md`, `DEVELOPMENT.md` and `deploy/nats/identity.conf` describe the sequence column, the command and the publish allow (documentation and configuration; no failing test).

## 4. Gate

- [ ] 4.1 `cargo fmt --all -- --check`, clippy with `-D warnings`, the 850-line check, `cargo test --workspace --locked`, `cargo deny --locked check`, `openspec validate --all --strict` and `openspec validate --archived` pass locally.
