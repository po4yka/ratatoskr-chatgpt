## Context

XR-021 CONTRACTS.md is the single source of truth. This change implements the ChatGPT half of its Round 2 amendments and records only decisions that are local to this repository.

## Decisions

- The receipt router moves wholesale to the new listener, including the tenant-token `POST /exports` route that shares it. Splitting it would add a second mount condition for no contract reason. The mount condition is unchanged: a database and a staging root. Without them no receipt listener is bound.
- Configuration copies the Claude shape: loopback only, nonzero port, and a violation naming `RATATOSKR__RECEIPT__LISTEN_ADDRESS` when it equals the operator address.
- The media type check compares the raw header value with `ARCHIVE_MEDIA_TYPE` before any streaming. `ArchiveReceiver::receive_platform_archive` keeps its own parse as a second line of defence.
- The size guard lives in `build_import_facts`, where the envelope is already serialized to compute the outbox body. The guard result travels back to the report assembly as a count, and the report is built in the same transaction.
- `publish_row` reads the connected server `max_payload` from the client's server info. A body larger than `max_payload - 1024` is permanent: the row gets `undeliverable_at` and `last_error = 'payload_too_large'`, the counter increments, and the pass continues. Broker unreachable, publish error and ack timeout stay transient and never set the column.
- `blob_lock` is `pg_advisory_xact_lock(hashtextextended('blob:' || digest, 0))`. Order is tenant lock, then digest locks ascending. The eraser re-checks and erases in one transaction per digest so the lock is held until the erase and the item update commit. The registrar takes the lock in the transaction that inserts the first referencing row and checks `BlobStore` presence. If the blob is missing it re-stores from the staged bytes it still holds, and otherwise fails with a retryable class and registers nothing.
- A blocking fault seam on the eraser (after its re-check, before commit) lets the test drive the interleavings with two connections.

## Risks

- The registrar holds a digest lock while it checks the blob store; the critical section is a stat and, rarely, a small re-store.
- Existing development databases must be recreated from the edited `schema.sql`; there are no migrations.
