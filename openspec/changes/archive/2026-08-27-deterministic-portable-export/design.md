## Context

See proposal.md and `portable-archive-export/spec.md`. The current parser and
reconciler expose normalized evidence in memory while the schema has owned
projection tables but no tenant-scoped read model. A portable command therefore
needs a small persistence seam as well as deterministic rendering.

## Goals / Non-Goals

**Goals:**

- Persist and load the normalized snapshot evidence required by the exporter under
  the existing first-version schema and account scope.
- Produce a reproducible ZIP that is useful without the service: canonical JSON,
  Markdown, assets, a verification manifest, and provenance headers.
- Verify each copied asset through `BlobStore` before it enters the ZIP.

**Non-Goals:**

- Provider import of the produced ZIP, browser/UI flows, a shared blob service,
  migrations, or deleting evidence based on a filter or absent snapshot.

## Decisions

### One archive member order and fixed ZIP metadata

The exporter materializes all selected members, sorts their sanitized paths by UTF-8
bytes, computes member digests, then appends `manifest.json` last. ZIP entries use the
stored method and a fixed DOS timestamp and permissions, avoiding compressor-version,
clock, and filesystem metadata variation. JSON maps are recursively canonicalized before
serialization; record arrays are sorted by stable external ID. Markdown has an HTML
comment provenance header so it stays readable.

The alternative, a timestamped ZIP or directory export, would not meet the byte-level
acceptance contract.

### Repository seam owns tenant filtering

`PortableArchiveRepository` loads an account's persisted normalized snapshot rows with
the project and inclusive observed-time predicates applied in SQL. `Postgres...` is the
production implementation. The parser/reconciliation handoff persists a snapshot with
the account, raw-export digest, parser identity and observed time; this uses existing
owned tables, editing `schema.sql` in place only if a missing column is required.

An exporter that filters reconstructed output after loading every tenant would make
cross-tenant disclosure too easy, so it is rejected.

### Asset paths are generated, never provider filenames

Asset output paths derive from a stable asset external ID and the SHA-256 digest; provider
display names remain JSON/Markdown data only. This prevents traversal/collisions while
retaining human labels. `BlobStore::verify` supplies the source path and verifies owner,
length, and digest before copying.

## Risks / Trade-offs

- [Archive size requires memory while assembling deterministic members] → Apply the
  existing archive byte limits and generate only selected content; future streaming
  determinism can preserve the member ordering contract.
- [Legacy data lacks a persisted normalized snapshot] → Export a valid, explicitly empty
  selection rather than inventing records; new normalization persistence is used for all
  subsequent state.
- [Raw provider content is sensitive] → Do not log titles, paths, or content; the output
  remains user-directed and tenant-scoped.

## Migration Plan

Deploy the schema definition and code together. No migration file is created under the
development-status rule. Rollback removes the command and exporter code; existing raw
evidence and normalized rows remain untouched.
