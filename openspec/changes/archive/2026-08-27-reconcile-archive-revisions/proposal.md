## Why

The synthetic parser produces isolated, loss-aware snapshots, but the archive
cannot yet tell an owner whether repeated exports describe the same
conversation, changed evidence, a missing observation, or an internally
inconsistent graph. Reconciliation and a conservative completeness report are
needed before parsed snapshots can become trustworthy archive history.

## What Changes

- Add a deterministic, in-memory reconciliation boundary that accepts ordered
  parsed archive snapshots and produces append-only conversation/message
  revision chains keyed by stable provider identity and content digest.
- Record an explicit `MissingFromLatestSnapshot` observation when a previously
  observed conversation does not occur in a later consumer export; do not
  infer provider deletion or remove historical evidence.
- Validate parent relationships per conversation, retain orphan messages, and
  surface each graph inconsistency as a structured warning rather than silently
  dropping or reparenting it.
- Produce per-archive and cumulative completeness reports with discovered
  counts, gaps, parser/graph warnings, and revision/deletion-observation
  statistics.
- Add minimized synthetic export sequences and test-first coverage for revision
  chains, unchanged deduplication, missing observations, orphan policy, and
  report arithmetic.

## Capabilities

### New Capabilities

- `archive-reconciliation`: reconciles repeated parsed snapshots into
  append-only identity and revision evidence without absence-based deletion.
- `archive-completeness-reporting`: reports archive-local and cumulative
  coverage, warnings, gaps, and reconciliation statistics conservatively.

### Modified Capabilities

- None.

## Impact

- Affected code: the `ratatoskr-chatgpt-archive` Rust library, new
  reconciliation model/module, public exports, and synthetic parser fixtures
  and integration tests.
- No database migration, SQL write path, asset-byte handling, portable export,
  cross-repository event contract, Knowledge/search integration, browser
  automation, or provider-schema expansion is included.
