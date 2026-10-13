## 0. Plan

- [x] 0.1 Open this change (documentation; it cannot start from a failing test). Cites XR-021 CONTRACTS.md R6-03.

## 1. The array column

- [x] 1.1 RED: `tests/stream_sequence.rs` selects `published_stream_seqs` as `to_jsonb(o) -> 'published_stream_seqs'`: `a_published_row_records_the_sequence_of_its_stream_message` and `a_second_publication_inside_the_duplicate_window_adds_no_element` fail on their assertions because the column does not exist.
- [x] 1.2 GREEN: `schema.sql` column `published_stream_seqs BIGINT[] NOT NULL DEFAULT '{}'`, `mark_published` merges ascending without duplicates, `erasure.rs` reads `unnest(published_stream_seqs)`, every test that named the old column moves to the new one without weakening an assertion.

## 2. The mark must land

- [x] 2.1 RED: unit test in `receipt/outbox/marks.rs` against real PostgreSQL: `mark_published` for a missing or already published row returns `PublishError::MarkLost` and increments `chatgpt_archive_outbox_mark_lost_total` (a signature-only stub of the variant that is never returned makes it fail on the assertion).
- [x] 2.2 GREEN: `rows_affected() == 1` is checked, `MarkLost` is returned and counted, and `publish_rows` reports it as a pass failure.

## 3. Serialize with the deletion

- [x] 3.1 `PublishProbe` (`receipt/probe.rs`), `ArchiveEventOutbox::with_publish_probe`, and the split of `receipt/outbox.rs` into a module directory (test seam and refactor; no behaviour, so no failing test).
- [x] 3.2 RED (a): `tests/stream_erasure_in_flight.rs` `a_fact_deleted_before_the_lock_is_never_published` fails because the pump publishes the deleted fact.
- [x] 3.3 RED (c): `a_deletion_inside_the_acknowledgement_window_waits_and_erases_the_copy` fails because the deletion completes at once and enqueues no command for the acknowledged copy.
- [x] 3.4 GREEN: first publication runs in a per-row transaction with `FOR UPDATE SKIP LOCKED`, the JetStream timeout is `PUBLISH_TIMEOUT` (5 s), the pool consequence is commented.

## 4. A PubAck whose commit failed is kept

- [x] 4.1 RED (b): `a_republish_after_the_duplicate_window_records_both_sequences` fails because only the latest sequence is recorded.
- [x] 4.2 RED (carried row deleted): `a_carried_copy_of_a_deleted_row_is_erased_by_the_pump` fails because the pump enqueues nothing.
- [x] 4.3 GREEN: the in-memory carry on `ArchiveEventOutbox`, the merge in `mark_published`, the reconcile step at the start of `publish_pending_once`, and `stream_erasure.rs` shared by the deletion and the pump.

## 5. The removal paths still work

- [x] 5.1 `tests/stream_erasure_request.rs` and the `privacy_deletion*.rs` suites pass against the array column; a new case shows one command naming both sequences of a republished fact removed by the export cascade.

## 6. Documentation

- [x] 6.1 `DEVELOPMENT.md` section "Stream erasure bookkeeping" and the README sentence (documentation; no failing test).

## 7. Gate

- [x] 7.1 `cargo fmt --all -- --check`, clippy with `-D warnings`, the 850-line check, `cargo test --workspace --locked`, `cargo deny --locked check`, `openspec validate --all --strict` and `openspec validate --archived` pass locally.
