## 0. Plan

- [x] 0.1 Open this change (documentation; it cannot start from a failing test). Cites XR-021 CONTRACTS.md R5-11.

## 1. Schema, resend state machine and acknowledgement SQL

- [ ] 1.1 RED: `tests/tombstone_ack.rs` (real PostgreSQL, fake transport is the real pump against a private broker) fails on its assertions with stub `deletion_ack::acknowledge` that changes nothing and a pump that never schedules a resend: a published tombstone has no `next_resend_at`, a due row is not selected for resend, an acknowledgement does not set `acknowledged_at`.
- [ ] 1.2 GREEN: the four columns in `schema.sql`, the pump's resend selection and bookkeeping, `deletion_ack::acknowledge`; the tests pass, including: an acknowledgement for another tenant's row or an unknown id changes nothing, an acknowledged row leaves the resend query, and a later deletion keeps an unacknowledged tombstone but removes an acknowledged one.

## 2. Consumer and supervision

- [ ] 2.1 RED: `tests/deletion_ack_consumer.rs` (private broker under the shipped `deploy/nats/identity.conf` stanza, durable provisioned as Edge does) fails against a consumer stub that acknowledges nothing: the first publish is not acknowledged, the same bytes with the same `Nats-Msg-Id` are published again after the injected `resend_after`, an acknowledgement published by the Knowledge identity stops the resend, a `refused` one stops it and increments `chatgpt_archive_tombstone_refused_total`, an acknowledgement for no row is acknowledged and ignored, an undecodable message is Termed, the gauges move.
- [ ] 2.2 GREEN: `deletion_ack_consumer.rs` and the service supervision; the tests pass and the service stays ready while Knowledge never answers.

## 3. Configuration

- [ ] 3.1 RED: `tests/config.rs` fails for the two keys (default 600 and 21600, range refusals, cap below the interval refused).
- [ ] 3.2 GREEN: `config.rs` parses `RATATOSKR__TOMBSTONE__RESEND_AFTER_SECONDS` and `RATATOSKR__TOMBSTONE__RESEND_CAP_SECONDS`; `services/chatgpt-archive/tests/boot.rs` still loads the shipped example.

## 4. Documentation and ACL copy

- [ ] 4.1 `README.md`, `DEVELOPMENT.md`, `deploy/systemd/chatgpt.conf.example` and `deploy/nats/identity.conf` describe the keys, the durable and the TRIO allows (documentation and configuration; no failing test).

## 5. Gate

- [ ] 5.1 `cargo fmt --all -- --check`, clippy with `-D warnings`, the 850-line check, `cargo test --workspace --locked`, `cargo deny --locked check`, `openspec validate --all --strict` and `openspec validate --archived` pass locally.
