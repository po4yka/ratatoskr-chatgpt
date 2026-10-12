## Why

XR-021 CONTRACTS.md R5-12. A deletion removes this service's outbox rows, but the conversation and project facts it already published carry full message text and stay in `ratatoskr_events` for up to 7 days (or until the 1 GiB cap). The deployed ACL gives no producer a stream-management permission, so nothing can remove them. Edge is the only identity allowed to erase a single message, on a producer's request.

## What Changes

- The pump stores the stream sequence of the `PubAck` of every published row in `published_stream_seq` (a duplicate PubAck carries the original sequence).
- Every code path that removes a published, content-bearing fact row from the outbox goes through one function, `drop_published_fact_rows` in `privacy_deletion`, which returns the sequences it deleted: the `outbox_event` inventory deletion, the export cascade rows and the tenant-scope removal in phase A. The removal of the facts of a deleted export covers the dropped facts of a survivor that is announced again.
- Phase A enqueues, in the same transaction, one `platform.stream_erasure.requested.v1` command per distinct owner (the row's envelope tenant), at most 500 ascending sequences per command, stored as a complete `CommandEnvelope` (producer `ratatoskr-chatgpt`, `aggregate_id` `privacy_request:<request id>`, `tenant_id` the owner, `correlation_id` the request's).
- The pump publishes the command to `cmd.platform.stream_erasure.requested.v1`; the closed type-to-subject function and the outbox `CHECK` learn the command type.
- A later deletion keeps an unpublished erasure command, like an unacknowledged tombstone.
- `deploy/nats/identity.conf` gains the publish allow `cmd.platform.stream_erasure.requested.v1`.

## Impact

Touches `schema.sql`, `crates/chatgpt-archive/src/{receipt/outbox.rs, stream_erasure.rs, privacy_deletion/*}`, `deploy/` and the README. Cites XR-021 CONTRACTS.md R5-12 rules 2 and 3. Edge's handler, the Platform ACL and Claude's equivalent are other repositories' work. Residual risk named: a fact published to the broker but not yet marked published when the deletion removes its row has no recorded sequence and is not erased.
