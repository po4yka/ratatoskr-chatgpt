# Design: authenticated-archive-receipt

## Context

The scaffold owns typed configuration, telemetry, the admin plane, the error envelope machinery (`FailureKind` reserves routing kinds for exactly this change), the content-addressed `BlobStore` (streaming SHA-256, hard-link create-new publish, verify-on-read), and an embedded `schema.sql` whose `exports`/`import_runs` relations are planned but unpopulated. See proposal.md for motivation.

## Goals / Non-Goals

**Goals:**

- One streaming receipt pipeline that never holds a whole archive in memory.
- Durable, resumable import progress with non-regressing transitions.
- Explicit same/different-content outcomes on duplicate archives, scoped per tenant.
- Hostile-input rejection (oversize declarations, size bombs, truncation) that always leaves either clean nothing or a diagnosable durable failure — never a half-published object presented as evidence.

**Non-Goals:**

- Parsing, inspection, extraction, completeness, events, portable export (plan items 3–5+).
- Platform device-token infrastructure, user sessions, or browser flows.
- Cross-repository upload contracts (the workspace store's `blob-references` spec is cited, not changed).

## Decisions

### D1. Tee-to-staging receipt, publish after digest

The body stream is consumed chunk-by-chunk; each chunk updates the SHA-256 hasher and is appended to a staging file under an isolated staging root (directory mode 0700, files 0600, names derived from the minted run UUID only — never client-supplied paths). The digest is known when the stream ends, so the duplicate check runs **before** any bytes are published; duplicates discard staging and answer immediately. Non-duplicates are published by streaming the staging file through `BlobStore::store` (its own incremental hashing cross-checks our digest), then one transaction records the export row and advances the run.

*Alternatives:* buffering in memory (rejected: unbounded memory on hostile input); store-first-then-dedupe (rejected: wasted disk I/O for every duplicate and a larger orphan window); hashing via a second network pass (impossible — the body arrives once).

### D2. Run row exists before the upload streams

After authentication succeeds, the receiver mints the import run (`received`) with its staging filename recorded, before consuming the body. Every later crash therefore has a durable anchor: mid-stream death leaves `received` plus a partial staging file; death after hashing leaves `hashed` plus verifiable bytes; death between blob publish and row recording leaves `hashed` whose resume re-publishes idempotently (content-addressed create-new converges to the same reference) and then completes the transaction. This is what makes "import failure must leave raw archive and a durable diagnosable run state" true for receipt-stage failures.

### D3. Guarded compare-and-set transitions

Stage changes execute as `UPDATE ... WHERE id = $1 AND state = $expected` inside the caller's transaction; zero rows updated means the expectation was stale and the transition is refused. Terminal states accept nothing. This makes replayed or racing commands safe without locks or leader election, satisfying the AGENTS.md rule that out-of-order/replayed commands cannot regress terminal state.

### D4. Per-tenant digest uniqueness; byte dedup stays in BlobStore

`exports` gains `UNIQUE (account_id, sha256_hex)` (replacing the global digest unique) and `account_id` becomes `NOT NULL`. Two tenants may hold the same bytes as separate exports — retention and deletion are per-tenant — while the blob layer deduplicates storage globally by content address. A duplicate receipt terminates its run in the explicit `duplicate` terminal state naming the existing export.

### D5. Authentication v1: configured bearer-token map

`RATATOSKR__RECEIPT__TENANT_TOKENS` holds `<token>=<external-ref>` pairs; each token authenticates to exactly one personal-kind account (upserted on first receipt). The authentication function sits behind a small trait seam so Platform-issued device tokens can replace the map later without touching handlers or tests. *Named limitation (stopgap, not a design goal):* until Platform integration lands there is no token rotation, no per-token audit identity beyond the account ref, and workspace-kind accounts cannot be expressed; documented in README when Platform work begins. Tokens live in config as secrets and render redacted everywhere.

### D6. New public failure kind `PayloadTooLarge`

Oversize is client-visible and actionable, so it joins the closed `FailureKind` table (413, `chatgpt.request.too_large`, not retryable as-is) rather than overloading invalid-request. The `ALL` inventory array grows with it; the exhaustive compiler checks do the rest. Missing acquisition/media-type headers reuse `InvalidRequest`; authentication failures reuse `Unauthenticated`.

### D7. Repository seam with hand-written fake

`ReceiptRepository` (create run, load run, guarded advance, find export by `(account, digest)`, record export) is a trait; production implements it with runtime-checked sqlx queries (consistent with the codebase's no-macros choice), tests use a hand-written `FakeReceiptRepository` in the existing `test-support` feature. Receiver logic is fully testable without PostgreSQL; the SQL correctness is proven separately against the compose database behind `CHATGPT_TEST_DATABASE_URL`, matching how `persistence_schema.rs` already works.

### D8. Single listener, routes mounted beside admin

The receipt router mounts onto the existing loopback-bound listener next to the admin plane. Receipt is a local data-plane surface for the export agent today; a separate bind address would add deployment surface without a consumer. When staging root is unconfigured the public surface simply does not mount (admin unaffected) — the boot contract test keeps passing with its minimal environment.

### D9. Startup sweep resumes interrupted runs

At boot, after schema application, the service sweeps runs left non-terminal by a previous process and attempts library-level resume: intact staging evidence advances them; lost evidence fails them durably. Resume is also exposed as a library function for tests and future retry commands. Concurrent-safety comes free from D3's guards.

## Risks / Trade-offs

- [Staging disk fill from concurrent uploads] -> the archive cap bounds each staging file; uploads are authenticated; startup sweep clears completed staging promptly; retention policy for abandoned parts is deferred with explicit TODO(author) note in the sweep.
- [Orphaned blob between publish and row recording] -> harmless by construction: unreferenced content-addressed object; resume re-publishes to the same reference; a future retention job may collect unreferenced objects.
- [Token map leaks through config mistakes] -> secret typing + redaction rules already enforced and tested; violations report value-free.
- [Single-listener exposure] -> loopback bind preserves the current trust boundary; moving receipt to its own address is a wiring change, not a contract change.
- [Schema edited in place under dev status] -> any database built from older `schema.sql` is disposable by policy; test databases are created fresh from the definition.

## Migration Plan

Development status: no migrations, no deployed databases to preserve. Land schema edits and code in the same change; test databases are recreated from `schema.sql`. Rollback = revert the branch; no data carries obligations across the boundary.

## Open Questions

None. Deferred deliberately: staging-part garbage collection thresholds and unreferenced-blob collection belong to the future retention item; they do not affect this change's contracts.
