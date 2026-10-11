## Why

Round 2 of changeset XR-021 fixes the defects that independent verifiers found by tracing each flow through the pushed code. Five of them land in the ChatGPT archive service. The binding text is XR-021 CONTRACTS.md sections R2-01, R2-15, R2-04, R2-07 and R2-10 (Round 2 amendments) and S06 D1; this proposal cites them and does not restate them.

- The Platform receipt is merged into the operator router, so Edge's probe of `127.0.0.1:8096` is refused and the provider never becomes ready with the default configuration (R2-01).
- The Platform receipt accepts any parseable media type, but the contract says `application/zip` exactly (R2-15, S06 D1).
- A conversation whose `ai_archive.conversation.*` envelope is larger than the broker limit is enqueued, refused on every attempt, and keeps the service not ready forever. The import report also says nothing about the fact that never reached the library (R2-04).
- Blob erasure re-checks references under the tenant lock only, so an import of identical bytes by another tenant can register a reference to a blob the eraser is about to delete (R2-07).
- No operator example ships, and the README names none of the bus, receipt and size-guard variables (R2-10).

## What Changes

- Add `RATATOSKR__RECEIPT__LISTEN_ADDRESS` (default `127.0.0.1:8096`, loopback only, refused when equal to the operator address) and bind the receipt router on its own listener. The operator router no longer serves the receipt or capability routes.
- Compare the Platform receipt `Content-Type` with `ARCHIVE_MEDIA_TYPE` exactly.
- Add `RATATOSKR__RECEIPT__MAX_EVENT_BYTES` (default 8000000, accepted 65536..=8388608). A fact whose envelope exceeds it is not enqueued and the terminal operation report is `partially_succeeded` with warning `ai_archive.facts.too_large`.
- Add `outbox_events.undeliverable_at`. The publisher compares each body with the connected server `max_payload` minus 1 KiB, marks an over-size row undeliverable with `last_error = 'payload_too_large'`, continues with the next rows, and never flips readiness for it. Counter `chatgpt_archive_outbox_undeliverable_total{class}`.
- One blob lock protocol: `blob_lock(tx, digest_hex)`. The eraser holds the digest lock for re-check and erase in one transaction. Every registrar of a blob reference takes it before inserting the first referencing row and verifies presence, re-storing from its staged bytes or failing with a retryable class.
- Ship `deploy/systemd/chatgpt.conf.example`, loaded by a test, and list the bus, receipt and size-guard variables in `README.md` and `DEVELOPMENT.md`.

Breaking, with no shims: the receipt and capability routes leave the operator port; `schema.sql` changes in place (`outbox_events.undeliverable_at`); a Platform receipt with any media type other than `application/zip` is refused.

## Impact

Touches configuration, the service process boundary, receipt HTTP, the fact projection and report assembly, the outbox pump, privacy deletion, the importer, `schema.sql` and documentation. The tenant-token receipt route, the deletion command interface, tombstone owner mapping, account erasure transport and the workspace profile are out of scope.
