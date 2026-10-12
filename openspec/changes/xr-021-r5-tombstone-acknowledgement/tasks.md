## 0. Plan

- [x] 0.1 Open this change (documentation; it cannot start from a failing test). Cites XR-021 CONTRACTS.md R5-11.

## 1. Schema, resend state machine and acknowledgement SQL

- [x] 1.1 RED: `tests/tombstone_ack.rs` (real PostgreSQL) and `tests/tombstone_resend.rs` (the real pump against a private JetStream broker whose events stream has a one second duplicate window) failed on their assertions with a stub `deletion_ack::acknowledge` that changed nothing and a pump that never scheduled a resend: a published tombstone had no `next_resend_at` (`a resend is scheduled`), a failed republish was not an error, applied and refused acknowledgements returned `Unmatched`, and a later deletion removed a published but unacknowledged tombstone. The negative cases (other owner, unknown id, duplicate, acknowledged row never published again) passed already and stay as guards.
- [x] 1.2 GREEN: the four columns in `schema.sql`, the pump's resend selection and bookkeeping, `deletion_ack::acknowledge`; the tests pass, including: an acknowledgement for another tenant's row or an unknown id changes nothing, an acknowledged row leaves the resend query, and a later deletion keeps an unacknowledged tombstone but removes an acknowledged one.

## 2. Consumer and supervision

- [x] 2.1 RED: `tests/deletion_ack_consumer.rs` (private broker under the shipped `deploy/nats/identity.conf` stanza, durable provisioned as Edge does) and `tests/tombstone_gauges.rs` failed against a consumer stub that acknowledged nothing: the resend never stopped (`the same bytes are published again`, then the acknowledgement was never recorded), a `refused` acknowledgement was never recorded or counted (`timed out waiting for the refusal to be recorded`), the unknown id and the undecodable message stayed pending (`every message must leave the durable's pending set`). The same test then passes with the consumer: first publish, no acknowledgement, resend with the same `Nats-Msg-Id` and equal bytes, acknowledgement, stop.
- [x] 2.2 GREEN: `deletion_ack/consumer.rs`, the gauges and `services/chatgpt-archive/src/deletion_acks.rs` supervision; the tests pass. `services/chatgpt-archive/tests/deletion_acks.rs` runs the real binary against the authorized private broker: not ready while the durable is missing, ready once Edge provisions it, ready while Knowledge never answers (the gauge shows one unacknowledged tombstone), and the gauge falls to zero when the acknowledgement arrives. It was written with the supervision and would fail its first assertion without it (the check would not exist).

## 3. Configuration

- [x] 3.1 RED: `tests/config.rs` `the_tombstone_resend_keys_default_and_are_bounded` and `the_tombstone_resend_cap_may_not_undercut_the_interval` failed with both keys reported as `is not recognized`.
- [x] 3.2 GREEN: `config.rs` parses `RATATOSKR__TOMBSTONE__RESEND_AFTER_SECONDS` and `RATATOSKR__TOMBSTONE__RESEND_CAP_SECONDS`; `services/chatgpt-archive/tests/boot.rs` still loads the shipped example.

## 4. Documentation and ACL copy

- [x] 4.1 `README.md`, `DEVELOPMENT.md`, `deploy/systemd/chatgpt.conf.example` and `deploy/nats/identity.conf` describe the keys, the durable and the TRIO allows (documentation and configuration; no failing test).

## 5. Gate

- [ ] 5.1 `cargo fmt --all -- --check`, clippy with `-D warnings`, the 850-line check, `cargo test --workspace --locked`, `cargo deny --locked check`, `openspec validate --all --strict` and `openspec validate --archived` pass locally.
