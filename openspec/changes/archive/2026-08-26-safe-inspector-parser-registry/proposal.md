## Why

Authenticated receipt now preserves raw export bytes, but those bytes cannot
yet move safely into any later import stage. A hostile ZIP must be inspected
and bounded before extraction, and parser choice must be explicit and
reproducible rather than inferred by an eventual concrete parser.

## What Changes

- Add a safe archive inspector that inventories ZIP entries, accounts for
  compressed and declared uncompressed sizes, detects entry types, and
  produces a structural detection input without executing or rendering entry
  contents.
- Add a bounded extractor that rejects unsafe paths, duplicate names,
  unsupported special entries, limit violations, and suspicious compression;
  it writes each accepted artifact directly to immutable BlobStore storage and
  records raw-export provenance.
- Add a versioned parser registry with capability declarations, structural
  selection, deterministic ambiguity handling, and an explicit unsupported
  outcome. No concrete ChatGPT schema parser is introduced.
- Extend runtime limits with explicit archive entry, per-entry, total
  decompressed, and compression-ratio caps.

## Capabilities

### New Capabilities

- `safe-archive-intake`: Inspects and extracts received archives under
  hostile-input limits while retaining accepted bytes by immutable BlobRef and
  raw-export provenance.
- `parser-registry`: Registers versioned parsers with declared capabilities and
  selects only a unique structural match, otherwise preserving an explicit
  unsupported or ambiguous result.

### Modified Capabilities

- `runtime-configuration`: Declares the archive-inspection and extraction
  limits used to bound hostile ZIP processing.

## Impact

- `crates/chatgpt-archive`: new archive inspection, extraction, and parser
  registry modules; configuration limits; integration tests with synthetic ZIP
  inputs and BlobStore evidence.
- `Cargo.toml`/`Cargo.lock`: a maintained ZIP reader dependency is required to
  read central-directory metadata and stream decompression without executing
  archive content.
- No database schema, HTTP endpoint, ChatGPT schema parser, Canvas/asset
  semantics, external contract, or browser/inference integration changes.
