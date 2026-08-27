## Context

See `proposal.md` for motivation. The service uses PostgreSQL 17 through SQLx 0.8.6, one process-owned pool, one repeatable `schema.sql`, a local content-addressed BlobStore, a declaration-only parser registry, and typed normalized tombstone outbox payloads. Receipt persists raw exports, but extracted artifacts and export-to-entity observations are not yet persisted; reconciliation remains in memory. Those provenance links are prerequisites for honest scoped deletion and reparse comparison and therefore enter the current schema in this change.

Changeset `AIARCH-009` expands the existing v1 tombstone reason with `user_requested` and deploys the compatible Knowledge consumer before this producer advances its contracts pin.

## Goals / Non-Goals

**Goals:**

- Make archive-, conversation-, and tenant-scoped erasure complete, replay-safe, content-free in audit, and downstream-safe.
- Run newer compiled parsers over verified retained raw bytes with one comparison path shared by dry-run and apply.
- Produce deterministic parser-migration and fixture-admission reports suitable for automation and golden tests.
- Keep the existing pool, runtime, dependency set, schema major, event major, and parser support claims honest.

**Non-Goals:**

- Provide atomic commit across PostgreSQL and filesystem namespaces; the design instead makes the cross-resource workflow durable and resumable.
- Add dynamic parser plugins, a real ChatGPT parser without owner evidence, browser/session automation, Compliance ingestion, Platform account-erasure coordination, or database migration tooling.
- Infer hard deletion from a newer parser omitting a record.

## Decisions

### 1. Persist the provenance needed to enumerate deletion closure

Edit `schema.sql` in place to add export-to-entity observations and extracted-artifact references, then add deletion requests/items/audit, reparse runs, and parser-migration reports. Every relation is tenant-owned or joins to tenant-owned evidence; uniqueness keys carry stable operation identity. No migration file, schema ledger, alternate version, or new pool is introduced.

The observation relation is authoritative for deciding whether normalized state has other retained raw provenance. `first_seen_export` and `last_seen_export` alone cannot prove that, and guessing from them would either leak deleted content or destroy retained evidence.

### 2. Serialize scoped writers with one tenant privacy gate

The transaction that creates a deletion inventory takes a tenant-scoped PostgreSQL advisory transaction lock, checks authorization, inserts the active request, and enumerates its immutable items in stable category/identity order. Receipt publication, normalized persistence, reparse apply, and deletion finalization take the same lock and refuse an overlapping active scope. This closes the race in which a new reference appears after enumeration.

The operation owner keeps each SQL transaction within one async scope and passes `&mut Transaction` to helpers. No connection is held during parsing, filesystem deletion, or report rendering. Unknown commit outcomes reconcile by request id; they are never blindly retried.

### 3. Use a durable three-phase cross-resource deletion

1. **Plan:** commit the authorized request and complete content-free inventory before destructive work.
2. **Purge:** erase each exact exclusive `BlobRef` idempotently; immediately before erasure, prove no retained database reference exists. Shared content is marked retained-shared. Failures keep the request resumable.
3. **Finalize:** once blob items verify absent/retained, persist a stable completion instant, write a deterministic non-sensitive audit-evidence blob, then run one database transaction that deletes selected source/normalized rows and old content-bearing outbox rows, appends the audit result, and inserts deduplicated `user_requested` tombstones referencing that audit blob.

This ordering cannot roll back already erased filesystem bytes, so a final-transaction failure leaves the durable request nonterminal and normalized rows temporarily present but inaccessible to new work. Retry reconciles by item state and request id. Completion is reported only after final commit.

Conversation deletion includes every raw export containing the conversation. Collateral normalized state survives only when another retained observation proves it. Tenant deletion enumerates every export and subject for that tenant. Equal bytes referenced by another tenant remain physically present.

### 4. Reparse through one immutable plan

Extend the parser registry with exact lookup and deterministic compatible-version discovery, keeping ordinary intake ambiguity-safe. A compiled parser executor boundary receives verified extracted evidence and returns the existing normalized parser result. The service registry contains only real compiled parsers; tests inject hand-written parsers to exercise version transitions without claiming provider support.

`ReparsePlanner` verifies raw bytes, reinspects and extracts under current limits, runs the exact newer parser, validates/reconciles, and compares against a projection fingerprint. It returns one immutable `ReparsePlan` containing the raw digest, registry fingerprint, input projection fingerprint, sorted changes/warnings/events, and report. Dry-run serializes this plan and performs no write. Apply requires those fingerprints still match, then persists through one transaction. A uniqueness constraint on export, target parser, raw digest, and input projection fingerprint returns the prior result on replay.

Omissions become coverage warnings, never deletion. The current runtime can plan `already_current` or `unsupported` until a genuinely newer compiled parser is registered; the engine and command are nevertheless executable and covered with two concrete deterministic parser implementations in integration tests.

### 5. Parser migration is deterministic orchestration over reparse

The migration planner lists one tenant's exports, sorts by stable export id, and classifies each exactly once. Apply invokes the same reparse engine only for eligible entries and continues after archive-local failure. Summary totals are reduced from final entries, never maintained independently, so totals cannot drift. Parser migration changes normalized revisions inside the current schema and is not database migration tooling.

### 6. Operator commands emit stable JSON without a new parser dependency

Extend the existing hand-written command parser with:

- `privacy-delete plan --tenant TENANT (--archive UUID | --conversation UUID | --all)`;
- `privacy-delete execute --tenant TENANT --request UUID --confirm`;
- `reparse --tenant TENANT --archive UUID --parser NAME@VERSION [--dry-run]`;
- `parser-migrate --tenant TENANT --parser NAME@VERSION [--dry-run]`;
- `fixture-admit --candidate PATH`.

Exactly one JSON document with sorted arrays goes to stdout. Diagnostics go to stderr. Exit `0` means completed plan/apply (including dry-run), `1` means operational failure or partial migration with a report, and `2` means invalid invocation/configuration. Destructive execution never prompts and requires `--confirm`; paths remain `PathBuf`. No ordinary argument carries credentials. Broken stdout pipe is success only after all requested state changes are already durable.

### 7. Reports are strict internal command contracts

Deletion, reparse, migration, and fixture-admission reports use tagged enums and strict human-authored manifest parsing. Report serialization has explicit snake-case field names and deterministic ordering; semantic invariants are established by constructors rather than accepting unchecked deserialized state. Private bodies, titles, filenames, raw values, source digests, and external account references never enter deletion reports or ordinary telemetry.

### 8. Owner fixture discovery separates private evidence from committed goldens

Document the process in `docs/testing/OWNER_FIXTURE_DISCOVERY.md`: receive explicit owner authorization; place the original only in an access-controlled private location; hash and inspect it with production hostile-input limits; record consent/provenance privately; derive minimal structural cases with synthetic identifiers/content; compare detector, variant inventory, relationship shape, unknown preservation, and completeness; run `fixture-admit`; obtain explicit owner review; then add only the approved derived fixture and non-sensitive manifest to `tests/golden/`.

The admission command rejects raw archives, unsafe paths, forbidden personal fields, missing reviews, nondeterministic expected output, or attempts to broaden the support matrix without a matching parser golden. Golden blessing remains explicit and diff-reviewed.

## Risks / Trade-offs

- [Filesystem deletion succeeds before final database commit] → durable item states, privacy gating, stable request ids, and resumable finalization; never report completion early.
- [Conversation deletion removes an archive containing unrelated records] → enumerate collateral impact and retain normalized state only when independent raw provenance exists.
- [Shared content address is deleted across tenants] → fresh reachability query under the privacy gate immediately before exact-path erasure.
- [Dry-run drifts before apply] → bind the plan to raw, registry, and projection fingerprints and refuse stale apply.
- [Generic reparse exists before a second production parser] → report `already_current`/`unsupported` honestly; never add a fake production version or claim real-export support.
- [Command JSON becomes an accidental public API] → document field/order/exit semantics, use golden process tests, and keep diagnostics off stdout.
- [Derived golden leaks owner data] → allowlist manifest fields, forbidden-field/content scans, explicit approval, and no raw archive acceptance.

## Migration Plan

1. Publish the `AIARCH-009` contract commit adding `user_requested`.
2. Publish the Knowledge consumer commit and verify scoped replay-safe deletion.
3. Advance this repository's contract pin, implement current-schema relations and lifecycle behavior test-first, and run PostgreSQL 17 integration tests plus the full gate.
4. Archive and sync this local OpenSpec change, integrate the task branch into `main`, push, then complete workspace verification.
5. Rollback before producer publication is a normal revert. After a privacy tombstone is published, keep contract/consumer support and disable only new producer operations; erased source or Knowledge data is not recreated.
