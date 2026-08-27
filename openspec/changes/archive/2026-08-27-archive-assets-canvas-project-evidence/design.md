## Context

See [proposal.md](proposal.md) and its spec deltas. The current synthetic parser
accepts only `conversations.json` bytes and returns in-memory conversation
records. `ArchiveExtractor` already publishes each safe archive entry as an
immutable `ExtractedArtifact` containing a verified `BlobRef`, provenance, and
quarantine flag. `BlobStore::verify` fails closed on owner, digest, length, or
media-type mismatch. Reconciliation currently tracks only conversations and
messages; its reports explicitly expose unobserved project and asset coverage.

Development status forbids migrations and parallel versions. This change keeps
the parser/reconciler in-memory like the preceding reconciliation work, so the
existing `schema.sql` is not edited and no database API is introduced.

## Goals / Non-Goals

**Goals:**

- Add one explicit synthetic archive input boundary that can combine structured
  JSON evidence and extracted artifact references without dereferencing an
  untrusted path or fetching a remote URL.
- Preserve typed project/instruction, Canvas, and asset evidence with stable
  identifiers, ordered content, raw-field retention, parser stamps, and
  deterministic reconciliation/reporting.
- Associate a usable `BlobRef` only after the existing BlobStore verifies it
  and the provider declaration matches it exactly; retain all anomalies as
  non-sensitive status/code evidence.

**Non-Goals:**

- Persistence, migrations, portable export, external contract/event changes,
  blobs owned by another service, external asset retrieval, preview/rendering,
  OCR, HTML/code execution, malware scanning policy, or a claim of real
  provider-schema support.
- Reclassifying existing archive-media quarantine policy. A pre-quarantined
  extraction stays unavailable to this feature; a text fixture supplies the
  positive verified-byte case.

## Decisions

### D1. Replace the byte-only synthetic parser input with an archive-evidence boundary

Replace the current public byte-only parse entry point and update every caller
and test to pass a `SyntheticArchiveInput`: selected parser ID, required
`conversations.json` bytes, optional `projects.json` and `canvas.json` bytes,
and extracted artifacts indexed only by normalized archive-relative path. The
operation becomes async because it verifies candidate BlobRefs through the
existing BlobStore before producing asset evidence. The registry declaration
still requires `conversations.json`; optional known files contribute no
ambiguous selection signal.

The documented synthetic grammar remains deliberately invented and clearly
version-stamped. `projects.json` is an array with project IDs, metadata,
ordered `instructions`, `conversation_ids`, and `asset_ids`; `canvas.json` is
an array with document IDs, optional project/conversation IDs, ordered inert
content, and metadata. File and generated-asset references occur in an optional
`assets.json` array, and content parts may name an `asset_id`. An asset declares
`id`, `kind` (`uploaded` or `generated`), optional
owner IDs, display metadata, optional `archive_path`, media type, byte length,
and SHA-256. Unknown fields use the same stable JSON-pointer raw-record policy
as existing parser values.

An alternative of inferring files from arbitrary JSON fields would silently
promote unknown provider data into a security-sensitive asset contract. An
alternative of adding a second parser leaves selection under-specified because
the current registry has only basename signals.

### D2. Keep bytes behind `BlobRef` and verify before association

Build a private `AssetVerifier` seam around `BlobStore::verify`, injected into
the parser input so integration tests can use the real local BlobStore and
small focused tests can use a configurable fake. For an asset with an
`archive_path`, locate exactly one extracted artifact by normalized path. It is
`Verified` only when the extraction was not quarantined, `verify` succeeds,
the BlobRef uses SHA-256, and its digest, length, and media type equal every
corresponding provider declaration. The result retains that `BlobRef` and raw
provenance without reading bytes into the normalized model.

Any missing entry, extraction quarantine, verification failure, conflicting
path, malformed declaration, or declaration mismatch creates a retained
`Quarantined` asset with one stable anomaly code and no usable BlobRef. An asset
with no archive path is a retained `Missing` reference rather than a failure or
quarantine. The parser neither downloads a URL nor tries to calculate a new
provider digest from untrusted metadata. This composes with the workspace
`blob-references` contract and avoids treating a BlobRef as a blob-serving API.

The alternative of storing/re-hashing candidate bytes in the parser duplicates
BlobStore ownership and makes the parser a byte-storage pathway. The alternative
of accepting an extracted BlobRef without verification violates the existing
fail-closed storage contract.

### D3. Model instructions and Canvas as inert, revisioned evidence

Introduce typed parsed records for `ParsedProject`, `ParsedInstruction`,
`ParsedCanvasDocument`, and `ParsedAsset`; every record retains its source
order, stable ID, relationships, provider metadata, and raw unknown values.
Instructions are separate ordered records so a system prompt cannot be lost in
a generic project JSON value. Canvas content is represented as supplied JSON or
text values only; no renderer, interpreter, MIME sniffer, or blob association
is invoked for it.

The alternative of placing instructions or Canvas into opaque project metadata
would make their first-class relationships, ordered evidence, and revisions
unobservable. The alternative of treating Canvas as a normal file would falsely
claim bytes where the export only supplies structured content.

### D4. Generalize append-only reconciliation by evidence kind

Extend the reconciler's deterministic SHA-256 canonicalization with separate
histories for projects, instructions, Canvas documents, and assets. Each
history has the same revision/reuse/present/missing semantics as conversations;
the public result keeps per-kind collections rather than overloading a
conversation ID namespace. A project omission records project and child
evidence omissions only where that identity was previously observed; it never
creates a tombstone. Asset availability status and BlobRef are part of the
revision digest, so a later verified association appends evidence rather than
rewriting a prior missing/quarantined observation.

The alternative of keeping only a current project view would violate the
archive's snapshot/revision contract. The alternative of using an untyped
generic record history loses ownership and status invariants that reports need.

### D5. Expand conservative reports without private values

Add structured per-archive and cumulative counters for projects, instructions,
Canvas documents, asset references, verified assets, missing assets, and
quarantined assets. Add coverage gaps for unobserved project/Canvas/asset
categories and report partial coverage whenever any category is unobserved or
an observed asset lacks verified bytes. Warning/anomaly codes, not asset names
or contents, are exposed. All vectors and maps are sorted in the existing
stable identity/sequence ordering before forming results or digests.

The alternative of reporting only a boolean asset success loses the distinction
between reference-only exports and detected integrity anomalies, which is
material to recovery decisions.

## Risks / Trade-offs

- [The invented grammar differs from a real export] → fixtures and documentation
  remain explicitly synthetic; an owner-authorized, reduced real fixture must
  drive a later parser change.
- [An extracted media entry is already quarantined] → it remains unavailable;
  this change does not weaken safe-intake policy to force a positive image case.
- [Asset metadata can contain sensitive strings] → parser errors, reports, and
  anomaly codes carry only structural identifiers/statuses and tests use
  invented minimal data.
- [Canonicalization can miss a new evidence field] → every new revision test
  changes one field per evidence kind and asserts an appended revision.
- [A broad parser input could invite arbitrary path reads] → artifact lookup is
  exact against extractor-provided normalized paths only; source JSON paths are
  never opened from disk.

## Migration Plan

No stored data or production rollout changes. The old in-memory parse API is
replaced in one first-version source change and every in-tree caller is updated.
Rollback reverts the parser input/model/reconciler changes together; raw archive
BlobRefs and existing conversation evidence remain untouched.

## Open Questions

None. Real provider field names and additional asset categories are explicitly
outside this synthetic, fixture-driven change and require their own evidence.
