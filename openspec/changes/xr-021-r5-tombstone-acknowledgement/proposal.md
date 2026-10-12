## Why

XR-021 CONTRACTS.md R5-11 (the owner's stream policy). `ratatoskr_events` keeps dropping its oldest message at 7 days or 1 GiB, and the pump marks a tombstone `published_at` at the PubAck and never looks at it again. A tombstone that Knowledge does not consume in time is lost silently and Knowledge keeps the derived conversations, projects and search documents for good. Nothing tells the service that Knowledge applied the deletion.

## What Changes

- `chatgpt_archive.outbox_events` gains `acknowledged_at`, `ack_outcome`, `resend_count` and `next_resend_at` (only tombstone rows use them).
- When the pump marks a tombstone published it sets `next_resend_at = now() + resend_after`. It also selects published, unacknowledged tombstones whose `next_resend_at` has passed and publishes the stored bytes again, unchanged, with the same `Nats-Msg-Id`; it then bumps `resend_count` and backs `next_resend_at` off to `least(resend_after * 2^resend_count, resend_cap)`. A failed republish uses the existing failure backoff. The resend never stops by count; it stops when `acknowledged_at` is set.
- A supervised consumer of the Edge-provisioned durable `ratatoskr_chatgpt_deletion_acks` (filter `evt.knowledge.deletion.acknowledged.v1`) validates each `knowledge.deletion.acknowledged.v1` envelope and sets `acknowledged_at` and `ack_outcome` on the tombstone row whose `event_id` equals `acknowledged_event_id` and whose tenant equals `owner`. A duplicate changes nothing, an acknowledgement for no row is acknowledged and ignored, an undecodable message is Termed. A `refused` outcome also stops the resend, logs an ERROR line and increments `chatgpt_archive_tombstone_refused_total`.
- A deletion never removes an unacknowledged tombstone row (the rule for an unpublished tombstone widens to an unacknowledged one).
- Two strict keys, `RATATOSKR__TOMBSTONE__RESEND_AFTER_SECONDS` (default 600, 30..=86400) and `RATATOSKR__TOMBSTONE__RESEND_CAP_SECONDS` (default 21600, at least the first, at most 604800).
- Gauges `chatgpt_archive_unacknowledged_tombstones` and `chatgpt_archive_oldest_unacknowledged_tombstone_age_seconds`; readiness is not tied to them.
- `deploy/nats/identity.conf` gains the TRIO(EVT, `ratatoskr_chatgpt_deletion_acks`) publish allows.

Breaking, with no shims: a deletion keeps unacknowledged tombstones where it used to keep only unpublished ones, and the service now needs its Edge-provisioned durable to exist when the event bus is configured.

## Impact

Touches `schema.sql`, `crates/chatgpt-archive/src/{receipt/outbox.rs, deletion_ack.rs, deletion_ack_consumer.rs, config.rs}`, `services/chatgpt-archive/src/`, `deploy/` and the README. Cites XR-021 CONTRACTS.md R5-11 rule 4. The contract, the Knowledge acknowledgement and the Platform durable and ACL are other repositories' work.
