## Context

The existing synthetic parser returns deterministic, loss-aware
`ParsedConversations` values for one archive. It deliberately has no SQL
write path or cross-snapshot semantics. See `proposal.md` and the two change
specs for the required externally observable reconciliation and reporting
behaviour.

## Goals / Non-Goals

**Goals:**

- Turn an ordered sequence of parsed archives into deterministic, append-only
  identity/revision and observation evidence.
- Expose graph faults and coverage limitations without dropping provider
  evidence or making optimistic completeness claims.
- Keep the first boundary pure and in-memory so its invariants are proven
  independently of future receipt-to-parser orchestration and persistence.

**Non-Goals:**

- Storing revisions, reports, graph relations, or observations in PostgreSQL;
  a database migration is forbidden by the current development status.
- Adding project, asset, Canvas, membership, portable-export, event, or
  Knowledge support. The current parser has no project membership evidence, so
  its absence is an explicit report gap rather than invented state.
- Inferring provider deletion, fetching missing evidence, or interpreting raw
  unknown content.

## Decisions

### D1. Reconcile an ordered immutable snapshot sequence at a pure public boundary

`ArchiveSnapshot` will bind a caller-supplied non-sensitive archive ID to the
parser/schema-stamped `ParsedConversations`. `ArchiveReconciler` will accept a
slice of snapshots in supplied chronological order and return a
`ReconciliationResult` with per-archive reports and a cumulative report.

The boundary will reject duplicate archive IDs and non-increasing sequence
positions before producing partial results. This avoids generated IDs and
clock-dependent ordering, making the result safely repeatable in tests and
ready for a later durable adapter. An incremental mutable API was considered,
but a sequence boundary makes history/absence semantics visible and avoids
pretending an in-memory call has stored evidence.

### D2. Use canonical normalized evidence hashes and separate revisions from observations

Conversation and message revision digests will be SHA-256 of a canonical,
explicit serialization of all normalized fields, provider metadata, ordered
parts, and relationship IDs. Input source order must not affect identity:
conversations and messages are sorted by external ID before digesting and
reporting, while content-part order remains evidence and participates in the
digest.

Each stable external identity owns an ordered vector of unique revisions. Each
archive receives a present observation referring to the selected revision, even
when its digest already exists; a new revision is appended only when that
identity has not previously produced that digest. This separates "seen again"
from "changed" and avoids both duplicate revisions and history loss.

### D3. Model absence and graph violations as evidence, not state repair

Before incorporating a snapshot, reconciliation validates each message parent
inside its conversation. Missing parents, self parents, and parent IDs found
only in another conversation become `Orphan` graph observations and structured
warning codes. The message stays fully represented and no artificial root is
created.

After processing present conversations, every identity previously observed but
absent from the current ConsumerExport gets a `MissingFromLatestSnapshot`
observation. No explicit deletion/tombstone variant exists in this boundary,
which makes absence-based deletion impossible by construction. Reappearing
conversations later receive a normal present observation and retain all prior
absence evidence.

### D4. Produce reports from reconciliation evidence, never raw content

Each `ArchiveCompletenessReport` is built from the just-processed snapshot and
contains counts, warning codes, coverage gaps, and revision-observation
statistics. The cumulative report aggregates the same evidence across all
archives, distinguishing unique conversations/messages, revisions, present
observations, and missing observations. Report warning/gap collections are
sorted by stable code and identity only; titles, message content, metadata,
filenames, and raw JSON cannot enter report values.

The synthetic schema has no project membership or asset-byte evidence. Reports
therefore declare `ProjectRelationshipsUnobserved` and
`AssetsUnobserved`, and use `StructurallyPartial`; no `Complete` result is
available from this parser path. Future parsers can supply positive coverage
evidence through a new change rather than changing this interpretation.

## Risks / Trade-offs

- [The pure result disappears after process exit] → it establishes stable
  semantics and tests first; a later receipt/persistence change can store these
  values without redefining them.
- [Canonicalization bugs could create false revisions] → use explicit serial
  digest inputs, sorted record collections, ordered parts, and fixture tests
  that distinguish changed from repeated evidence.
- [A known parser's omitted records can be ambiguous] → model only
  `MissingFromLatestSnapshot`, retain history, and forbid deletion inference.
- [The synthetic parser lacks project/assets evidence] → publish explicit gaps
  and a partial classification rather than claiming coverage.

## Migration Plan

No migration or live rollout is required. The change adds an in-memory library
contract. Rollback removes that module; it has not changed raw archives,
database state, or externally published events.
