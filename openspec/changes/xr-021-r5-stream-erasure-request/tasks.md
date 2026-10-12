## 0. Plan

- [x] 0.1 Open this change (documentation; it cannot start from a failing test). Cites XR-021 CONTRACTS.md R5-12.

## 1. Sequence recording

- [x] 1.1 RED: `tests/stream_sequence.rs` (real broker) failed on its assertions with a pump that stored no sequence: `a published row records its stream sequence` (the column was null) and `a duplicate acknowledgement carries the original sequence` (left None, right Some(1)).
- [x] 1.2 GREEN: `published_stream_seq` in `schema.sql`, `mark_published(id, sequence)` from the `PublishAck`; the tests pass.

## 2. The command

- [x] 2.1 RED: `tests/stream_erasure_request.rs` (real PostgreSQL, then a real broker with a fake Platform handler) failed with a stub `enqueue_erasure_commands` that queued nothing: an archive deletion queued no command (`one command for the one owner`, left 0), a tenant deletion listed nothing, 502 sequences were not split into two commands, the survivor path listed nothing, an unpublished erasure command could not be shown to survive, and the broker held no command on `cmd.platform.stream_erasure.requested.v1`.
- [x] 2.2 GREEN: `drop_published_fact_rows`, `stream_erasure.rs`, the pump subject and the schema `CHECK`; the tests pass, including the fake Platform handler that deletes the named sequence and leaves another owner's message alone.

## 3. Documentation and ACL copy

- [x] 3.1 `README.md`, `DEVELOPMENT.md` and `deploy/nats/identity.conf` describe the sequence column, the command and the publish allow (documentation and configuration; no failing test).

## 4. Gate

- [ ] 4.1 `cargo fmt --all -- --check`, clippy with `-D warnings`, the 850-line check, `cargo test --workspace --locked`, `cargo deny --locked check`, `openspec validate --all --strict` and `openspec validate --archived` pass locally.
