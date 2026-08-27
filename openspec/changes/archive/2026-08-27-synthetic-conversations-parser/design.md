## Context

See [proposal.md](proposal.md) for motivation and
[`synthetic-conversations-parser`](specs/synthetic-conversations-parser/spec.md)
for the behavioral contract. `ArchiveInspector` already provides a bounded
inventory and `ParserRegistry` makes exactly one `conversations.json` parser
selectable for a ConsumerExport, but there is no concrete parser or normalized
staging representation. The existing `chatgpt_archive` schema already names
the eventual tables; development status prohibits migrations.

## Goals / Non-Goals

**Goals:** expose a small public, in-memory parse result whose fields map
directly to conversation, message, and content-part schema columns; make the
synthetic source grammar precise and loss-aware; keep output equality and
ordering deterministic.

**Non-Goals:** SQL writes, import-run transitions, cross-snapshot graph
reconciliation, relationship validation, asset-byte storage, projects/Canvas,
real-export support, external contract changes, or automatic registry wiring
into an HTTP import flow.

## Decisions

### D1. Publish a narrow synthetic parser contract, separate from persistence

`SyntheticConversationsParser` will parse bytes from the already extracted
`conversations.json` artifact and return `ParsedConversations`: schema ID,
parser ID, ordered normalized conversations/messages/parts, and raw-preserved
records. It will not take a database connection or BlobStore. This keeps item 4
testable without pretending that parsing has reconciled a snapshot.

The normalized types will carry external IDs, optional title/timestamps/model
slug, parent external ID, role, ordinal, typed payload, and provider metadata.
They are deliberately ordinary deterministic values, not database IDs. The
alternative—writing the existing SQL tables now—would entangle this parser with
item 5's identity/revision decisions and would create unvalidated graph state.

### D2. Specify a small synthetic JSON grammar and reject malformed required shape

The committed fixture root is an array of conversation objects. Each object has
`id`, optional `title`, optional `create_time`/`update_time`, and `mapping`.
Every mapping value has optional `id`, optional `parent`, and optional
`message`; a message has `id`, `author.role`, optional times,
`metadata.model_slug`, and `content.parts`. A missing `message` is retained as
raw mapping metadata but does not invent a normalized message.

Part classification is explicit: JSON strings become text; objects with
`kind: "tool_call"` or `kind: "tool_result"` become tool parts; objects with
`kind: "media_reference"` become media-reference parts. Every other JSON value
becomes an unknown part with its original value. Media references stay metadata
only and cannot claim that a file or image was archived.

Serde typed envelopes plus flattened `serde_json::Map` values will validate the
required structure without a giant permissive `Value` pipeline. Parsing errors
will report only structural location/class, never raw conversation content.

### D3. Preserve consumed and unknown fields without duplication ambiguity

Each normalized record will contain `provider_metadata` with all fields not
consumed by its typed columns. Each part keeps its original JSON payload; an
unknown part also yields a raw-preservation record with a stable JSON-pointer
source path. Unknown conversation, mapping, message, author, content, and
metadata fields are emitted as raw-preservation records in source order. This
means reprocessing can recover original unknown values while callers can use the
typed projection. The parser never logs, renders, executes, or dereferences
these values.

### D4. Pin selection and parser stamps

The parser exposes one stable `ParserRegistration` for `ConsumerExport` and the
`conversations.json` inventory signal. Its parse result emits the exact parser
name/version from that registration and a synthetic schema ID. Tests select the
registration before parsing to prove the stamp and prevent a filename-only
unstamped parse path.

The current registry cannot distinguish two consumer parsers with the same
basename signal. It is safe for this first exclusive parser; a later competing
real-export parser must extend inspection evidence in its own OpenSpec change
before registration rather than overlapping declarations and accepting an
ambiguous result.

### D5. Use committed synthetic evidence and a private real-fixture gate

`tests/fixtures/synthetic_conversations.json` contains only invented data and
exercises text, tool, media-reference, metadata, and unknown values. Integration
tests read it through `CARGO_MANIFEST_DIR`, parse it twice, and compare complete
outputs without blessing mode or generated values. A supplied owner-authorized
real export stays outside version control and is reduced/redacted before any
test fixture is considered; until then, the real-fixture golden test is an
explicit follow-up blocker, not a passing claim or ignored test.

## Risks / Trade-offs

- [Synthetic grammar may differ from real provider exports] → label schema and
  parser as synthetic, scope its registry declaration narrowly, and require a
  separately observed real-fixture validation before production support.
- [Unknown JSON can be large or sensitive] → it remains in the in-memory parse
  result only, is never logged, and later persistence applies bounded raw
  record/blob policy.
- [Multiple parser declarations could become ambiguous] → preserve the
  registry's explicit ambiguity outcome; do not choose by filename guessing.
- [Timestamp variants could be under-specified] → accept documented numeric
  epoch seconds only and reject malformed required values rather than inventing
  a date interpretation.

## Migration Plan

No database migration or production rollout is needed: the change introduces
an in-memory parser contract only. Rollback removes the parser registration and
module; received raw archives and current schema stay unchanged.

## Open Questions

- The owner-authorized real fixture is unavailable. Its observed shape will
  determine a subsequent production schema/parser proposal; it cannot be
  inferred safely from this synthetic contract.
