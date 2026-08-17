# ChatGPT archive interfaces

## Inbound

Registered-device/Platform upload completion, import/retry/cancel, portable export, privacy delete, reparse, optional Compliance cursor/pull commands, and operation context.

## Outbound

Export received/imported/partial/failed, project/conversation/asset upsert, snapshot completed, completeness, privacy-deletion, and Knowledge indexing events plus safe progress/results.

## Internal boundaries

- `ArchiveInspector`: hash, limits, file inventory, provider/schema detection.
- `ParserRegistry`: schema/acquisition -> versioned parser.
- `StagingImporter`: isolated extraction and typed/unknown records.
- `Reconciler`: stable identities, graphs, revisions, assets, snapshot evidence.
- `PortableExporter`: deterministic manifest plus JSON/Markdown/assets.
- optional Compliance adapter with independent cursor/auth policy.

Errors distinguish invalid/unsupported archive, limits, parser/relationship/asset incompleteness, storage, privacy, and Compliance auth/transient failures without exposing content.
