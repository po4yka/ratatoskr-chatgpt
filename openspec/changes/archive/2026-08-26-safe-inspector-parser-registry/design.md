## Context

The receipt change already verifies and stores raw archive bytes behind a
BlobRef, then leaves an import run at `stored`. This change implements the
next internal boundary in `docs/INTERFACES.md`; it must not introduce a
provider schema parser or schema migration.

## Goals / Non-Goals

**Goals:**

- Inspect central-directory metadata before decompression and reject unsafe
  structure deterministically.
- Stream only accepted entry bytes through private staging into BlobStore.
- Return lossless structural evidence for a later concrete parser.
- Keep parser choice data-driven and explicit without adding a parser now.

**Non-Goals:**

- A ChatGPT export schema, message/content parsing, project/Canvas/asset
  semantics, database persistence for extracted artifacts, or HTTP wiring.
- Rendering, decoding, executing, malware-scanning, or otherwise trusting
  HTML, media, scripts, or files.

## Decisions

### D1. Use a maintained Rust ZIP reader with default features disabled

Add the maintained `zip` crate with only the Deflate reader enabled. It reads
central-directory metadata and streams entry decompression in-process without
shelling out, executing archive entries, or extracting them by path. A custom
ZIP/Deflate implementation is rejected because parser mistakes at this
security boundary are higher risk; invoking `unzip` is rejected because its
behaviour and installed version are host-dependent.

### D2. Inspect before any entry bytes are read

`ArchiveInspector` verifies the raw BlobRef, opens the ZIP read-only, and
creates `ArchiveInventory`. It normalizes names with a platform-independent
archive-path grammar, rejects absolute/parent/empty/duplicate paths and
special Unix entry modes, totals declared metadata with checked arithmetic,
and classifies entries from bytes only after hard bounds are established.
Structural signals are basename and type/count markers, not provider claims.

### D3. Extraction uses private per-run staging and repeats enforcement

`ArchiveExtractor` accepts a successful inventory, opens the verified raw
blob again, then streams each entry to a UUID-named file in an owned `0700`
directory. It rechecks actual byte totals while reading, streams the staging
file to BlobStore, verifies the returned BlobRef, and removes only that owned
staging file. No archive entry name becomes a filesystem path. A failure
before BlobStore publication returns no artifact result for that entry.

### D4. Classification is conservative quarantine metadata

Magic prefixes and bounded first bytes distinguish JSON, HTML, common image
formats, generic text, and unknown. HTML and media are represented as
quarantined references. The classifier neither parses HTML nor decodes media;
the first provider parser and item 6 own semantic meaning.

### D5. Registry selection is declaration matching, never filename guessing

`ParserRegistration` has `ParserId`, version, supported acquisition modes,
and a structural predicate over `ArchiveInventory`. `ParserRegistry::select`
collects all matches: exactly one yields `Selected`; zero yields
`Unsupported`; more than one yields `Ambiguous`. Registration uses the
identity/version pair as a write-once key. The registry has no parse method
until item 4 supplies a concrete parser contract.

### D6. Limits extend the existing closed configuration

The existing `Limits` type gains positive, documented defaults for maximum
entry count, per-entry decompressed bytes, aggregate decompressed bytes, and
compression ratio. `ArchiveLimits` is constructed from that configuration so
tests can use small values without mutating process environment.

## Risks / Trade-offs

- [ZIP library dependency and codec vulnerabilities] -> use one pinned,
  default-feature-minimized crate; the dependency is subject to Cargo.lock,
  cargo-deny, and the user approval required before it is introduced.
- [central-directory metadata lies] -> enforce every relevant byte limit both
  from declared metadata during inspection and while extraction reads bytes.
- [large staging use] -> per-entry and aggregate limits bound it; staging is
  private and entry names never select destinations.
- [no durable artifact rows yet] -> returned provenance keeps the raw digest
  link; item 6 or reconciliation will own persistence without retroactively
  inventing semantics.

## Migration Plan

No database schema changes occur under the current development status. Rollback
is a normal revert: raw receipt evidence stays untouched, and later import
work has no partial normalized state to clean up.
