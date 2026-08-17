# Ratatoskr ChatGPT Archive Agent Instructions

## Scope

These instructions apply to the `ratatoskr-chatgpt` repository.

This repository owns archival ingestion and local preservation of **ChatGPT product data**: exports, projects, conversations, messages, files, generated assets, and provider-specific records.

## Product boundary

ChatGPT product history is not the same system as OpenAI API Conversations/Responses state.

This repository must not:

- act as an OpenAI inference gateway;
- treat `/v1/conversations` as ChatGPT sidebar history;
- call GPT models as part of archive ownership;
- automate a consumer ChatGPT login/session to crawl history;
- store user passwords, MFA secrets, or browser cookies.

Inference and analysis belong to `ratatoskr-knowledge`; this service preserves and normalizes archive evidence.

## Repository mission

The service must:

- accept official ChatGPT export archives and supported compliance feeds;
- store original provider exports immutably before parsing;
- detect and version provider export schemas;
- reconstruct projects, conversations, message graphs, files, and assets without silently discarding unknown data;
- reconcile repeated snapshots conservatively;
- report exactly what was and was not present in each import;
- produce portable local exports that remain useful without Ratatoskr;
- publish stable archive contracts for Knowledge and clients.

## Current phase

The repository is in architecture bootstrap. Do not assume Rust crates, parsers, migrations, Compliance connectors, fixtures, or CI commands exist unless they are present in the checkout.

When creating initial implementation:

- acquire real user-authorized export fixtures privately and minimize/redact derived test fixtures;
- implement raw storage and safe archive inspection before normalization;
- preserve unknown records;
- avoid coding against an unofficial example as if it were a permanent provider schema.

## Sources of truth

Use this order:

1. active task/changeset and accepted ADRs;
2. `README.md`;
3. AI archive/event contracts from `ratatoskr-contracts`;
4. the immutable raw provider export or compliance event;
5. parser-version documentation and completeness report;
6. normalized projections;
7. implementation details.

When normalized state disagrees with the raw archive, the raw archive is the evidence and the parser/projection must be corrected or versioned.

## Acquisition modes

Represent acquisition explicitly, for example:

```text
ConsumerExport
EduExport
ComplianceLog
ManualConversationCapture
LegacyImport
```

Each mode has different authority and completeness.

Rules:

- Do not merge acquisition modes into an unlabeled generic import.
- Record provider account/workspace identity where available.
- Record received/imported/requested timestamps separately.
- Store export/log cursor IDs and parser versions.
- Do not claim that a consumer export is a live replica.
- Do not claim that a compliance log contains every Project asset unless verified by the actual API/data.
- Manual capture is partial by definition and must not participate in absence-based deletion.

## Hard bounded-context rules

### ChatGPT Archive owns

- ChatGPT account/workspace archive identity;
- immutable raw export snapshots and provider records;
- import runs, parser versions, warnings, and completeness;
- projects and project metadata observed in exports;
- conversations and message graphs;
- files, images, Canvas-like/generated assets, citations, and source references found in the archive;
- provider-specific unknown records;
- snapshot/revision/tombstone state;
- portable archive exports;
- ChatGPT-specific outbox/inbox records;
- references to Knowledge indexing/analysis.

### ChatGPT Archive does not own

- OpenAI inference API requests;
- general LLM analysis or embeddings;
- Platform user sessions/devices;
- local client collections;
- browser automation or consumer ChatGPT credentials;
- Claude archive data;
- BlobStore implementation outside the approved storage interface.

## Raw-first immutable storage

This is non-negotiable:

> Store the original provider archive or compliance record before destructive parsing or normalization.

Rules:

- compute SHA-256 while receiving/streaming when practical;
- use content-addressed or collision-safe BlobStore keys;
- record byte size, acquisition mode, provider metadata, and receive time;
- do not modify or rewrite the original ZIP/JSON;
- do not delete the only raw snapshot after successful import;
- verify stored bytes can be read and match the expected hash;
- keep raw storage access-controlled and excluded from ordinary logs;
- deduplicate identical archives by hash while retaining import/audit events as needed.

A normalized database is not a substitute for raw evidence.

## Safe archive intake

Treat every archive as malicious input, even when downloaded from the provider.

Mandatory controls:

- reject absolute paths and `..` traversal;
- normalize and validate every entry path;
- cap archive byte size, entry count, nesting, per-entry size, and total decompressed size;
- detect suspicious compression ratios/zip bombs;
- reject or safely handle symlinks, hard links, devices, and special files;
- never execute archive contents;
- never render active HTML during import;
- sniff MIME safely rather than trusting extensions;
- use isolated temporary directories with restrictive permissions;
- clean only owned temporary paths;
- treat filenames and embedded URLs as untrusted strings.

Import failure must leave the raw archive and a durable diagnosable run state.

## Schema detection and parser versioning

- Detect provider/export schema from observed structure, not only filename.
- Assign a stable detected schema identifier and parser version.
- Keep parsers versioned and testable against fixtures.
- Preserve unknown top-level files/sections/record variants as raw references where safe.
- Never silently drop a record because the current parser does not understand it.
- Parser upgrades create a new normalized revision/reprocessing run; they do not rewrite historical interpretation without evidence.
- Record parse warnings, missing relationships, unknown variants, and asset mismatches.

Avoid one giant permissive `serde_json::Value` pipeline that reports success without structural validation.

## Import state machine

Use explicit durable state, for example:

```text
received
stored
inspecting
schema_detected
extracting
staging
validating
reconciling
publishing
completed
partial
failed
quarantined
```

Rules:

- transitions are idempotent;
- retry resumes from safe evidence or restarts a documented phase;
- completion requires relationship and asset validation, not only JSON parse success;
- `partial` is a successful terminal class with warnings, not hidden `completed`;
- a failed normalized import does not invalidate the raw archive;
- out-of-order/replayed commands cannot regress terminal state;
- import runs retain correlation, acquisition, parser, and archive hash.

## Project model

A ChatGPT Project may include more than conversations. Preserve observed components separately:

- external project ID;
- title/description;
- instructions;
- chats;
- uploaded files and pasted text;
- saved responses and generated assets;
- Canvas-like documents;
- external Drive/Slack/app references;
- sharing/membership/visibility metadata when present;
- memory/configuration metadata when present;
- created/updated/archived/deleted observations.

Rules:

- absence of project instructions/files from a consumer export does not prove the project lacked them;
- external references are not locally backed up unless their bytes are present or another service verifies them;
- preserve `locally_backed_up`/completeness state per source;
- do not fetch external Drive/Slack resources with ChatGPT credentials;
- do not claim a project is fully restorable when required components are missing.

## Conversations are graphs

Do not model conversations only as `messages(position)`.

Support, when observed:

- stable conversation and message external IDs;
- parent-message relationships;
- edits and message revisions;
- regenerated assistant responses;
- branches and branch chats;
- system/tool/internal message roles;
- citations and tool calls/results;
- interrupted/incomplete responses;
- generated files and assets;
- model metadata and provider-specific fields.

Rules:

- preserve graph relationships even when a preferred linear branch is derived;
- a derived transcript is a projection, not canonical storage;
- detect cycles/orphans/duplicate IDs and report them;
- preserve unknown message/content variants;
- never discard alternate branches because the UI commonly shows one path.

## Content parts

Use typed heterogeneous parts such as:

```text
Text
Markdown
Image
File
Code
Citation
ToolCall
ToolResult
Artifact
Canvas
Unknown
```

Rules:

- preserve ordering within the message;
- separate file references from available local blob bytes;
- validate sizes, MIME, hashes, and ownership;
- store unknown variants safely;
- do not execute code/tool calls or render unsafe HTML;
- do not infer model/tool execution beyond provider evidence.

## Files and assets

- Hash and store available asset bytes content-addressably.
- Record provider filename, MIME evidence, size, external ID, and reference relationships.
- Treat missing referenced files as completeness warnings.
- Do not mark an asset archived when only a URL/reference exists.
- Keep generated images/files and uploaded user files distinguishable.
- Sanitize filenames for display; never use them as storage paths.
- Apply archive/file size and malware-scanning policy where configured.
- Preserve asset revision/version relationships when present.

## Snapshot and revision semantics

Every official export is a snapshot and remains independently identifiable.

Rules:

- repeated snapshots do not overwrite prior raw archives;
- changed projects/conversations/messages/assets create new observations or revisions;
- normalized current projections reference first/last seen snapshots;
- absence from a single snapshot does not prove deletion;
- explicit provider deletion evidence, compliance events, or repeated policy-defined reconciliation may create a tombstone;
- local hard deletion follows separate retention/user policy;
- losing access to a shared project is distinct from owner deletion;
- temporary chats may be absent and completeness must say so.

Use states such as:

```text
Present
MissingFromLatestSnapshot
ExplicitlyDeleted
AccessLost
Unknown
```

Do not collapse them into one `deleted` boolean.

## Completeness reporting

Every import produces a durable report, including where available:

- archive/schema/parser identity;
- projects discovered;
- conversations/messages discovered;
- branches/orphans/duplicates;
- files/assets referenced, archived, missing, or unknown;
- project instructions and membership metadata found;
- Canvas/generated assets found;
- unknown record variants;
- parse/relationship warnings;
- categories not present in the export;
- acquisition-specific limitations.

Representative status classes:

```text
Complete
ConversationsComplete
StructurallyPartial
AssetsPartial
Unknown
FailedValidation
```

Do not use `Complete` until the actual provider format and expected categories justify it.

## Compliance connector rules

If an Enterprise/Edu compliance adapter is added:

- use official supported authentication and least-privilege credentials;
- persist durable cursors/checkpoints only after records are stored;
- handle provider retention windows by continuous polling and observable lag;
- keep raw compliance events immutable or append-only;
- deduplicate event delivery;
- classify revocation, scope, rate-limit, and schema changes explicitly;
- reconcile compliance events with periodic snapshots where available;
- do not assume product Project/file coverage beyond verified fields;
- isolate organization/workspace ownership and access policy.

Compliance API credentials never enter export archives, clients, events, or logs.

## Portable local export

Portable export should remain readable without Ratatoskr, for example:

```text
project/
  project.json
  README.md
  instructions.md
  conversations/*.json
  conversations/*.md
  files/
  assets/
  manifest.json
```

Rules:

- include a manifest with source snapshot IDs, hashes, completeness, and exporter version;
- preserve graph JSON even when Markdown provides a linear reading view;
- never omit warnings/missing assets from the manifest;
- sanitize paths and prevent collisions;
- produce deterministic output where practical;
- do not include secrets or unrelated raw account data;
- distinguish provider-original bytes from generated Markdown/JSON projections.

Portable export is not a claim that it can be imported back into ChatGPT.

## Knowledge integration

Publish stable archive events/references containing:

- provider/account/project/conversation/message IDs;
- snapshot and revision IDs;
- content hashes;
- normalized typed content;
- provenance to raw export records;
- asset/blob references;
- completeness/availability state;
- operation/correlation IDs.

`ratatoskr-knowledge` owns summaries, embeddings, decisions, entities, and semantic search. Knowledge results never replace archive evidence.

## Persistence and migrations

ChatGPT Archive writes only its owned schema.

Conceptual data includes:

```text
chatgpt_accounts
chatgpt_exports
chatgpt_import_runs
chatgpt_projects
chatgpt_project_sources
chatgpt_conversations
chatgpt_messages
chatgpt_message_relations
chatgpt_content_parts
chatgpt_assets
chatgpt_revisions
chatgpt_tombstones
chatgpt_outbox
chatgpt_inbox
```

Rules:

- no cross-schema writes or foreign keys;
- raw provider records remain separable from normalized projections;
- stable provider IDs and snapshot/revision uniqueness are constrained;
- migrations preserve raw links, graph relationships, and completeness history;
- destructive cleanup cannot remove the only source evidence for normalized state;
- large bytes use protected BlobStore references.

## Commands and events

Representative messages include:

```text
chatgpt.export.received.v1
chatgpt.export.ingested.v1
chatgpt.project.upserted.v1
chatgpt.conversation.upserted.v1
ai_archive.asset.stored.v1
ai_archive.snapshot.completed.v1
ai_archive.snapshot.partial.v1
```

Use canonical contracts, transactional outbox, inbox deduplication, correlation/causation IDs, and at-least-once-safe handlers.

Events carry stable references and completeness metadata, not entire private archives.

## Prohibited implementation approaches

Do not add:

- automated ChatGPT web login;
- browser cookie/session storage or replay;
- undocumented consumer history endpoints as the supported path;
- password/MFA collection;
- browser scraping of every chat/project;
- OpenAI inference API state presented as ChatGPT history;
- automatic model analysis inside archive ingestion;
- deletion based solely on absence from one export.

## Security and privacy

- Treat exports, conversations, prompts, files, and assets as highly sensitive private data.
- Enforce internal-user/workspace ownership at every read/import/export endpoint.
- Encrypt credentials and sensitive storage where configured.
- Do not log message bodies, raw JSON, archive paths, tokens, user filenames, or generated content by default.
- Redact errors before user display.
- Limit raw archive access and retention.
- Never execute code, tool calls, HTML, macros, or files found in an export.
- Keep portable exports private by default and write them atomically.
- Audit imports, raw export access, portable exports, retention, and hard deletion.
- Use least-privilege database, network, and BlobStore roles.

## Observability

Required telemetry should cover:

- archives received/deduplicated/stored;
- schema/parser detection;
- import phase latency and terminal status;
- project/conversation/message/asset counts;
- graph validation warnings;
- unknown variants and missing assets;
- completeness classes;
- compliance cursor lag/rate-limit/reauth state;
- reprocessing/export operations;
- outbox/inbox lag and duplicates;
- correlation, archive, import-run, project, and conversation IDs in non-sensitive form.

Never place titles, message text, filenames, or account emails in ordinary metric labels.

## Testing expectations

When implementation exists, include applicable tests for:

- content hashing and duplicate archive handling;
- archive traversal, symlink, count, size, and zip-bomb defenses;
- schema detection and parser version selection;
- unknown record preservation;
- import state-machine idempotency/recovery;
- project/source/instruction relationships;
- conversation graph branches, edits, regenerated messages, cycles, orphans, and duplicates;
- heterogeneous content parts;
- asset hashing, missing references, and path safety;
- completeness classification;
- absence-without-deletion and explicit tombstones;
- compliance cursor/deduplication/retention-window behavior;
- portable export determinism and manifest correctness;
- outbox/inbox replay and migrations.

Use synthetic or heavily minimized/redacted fixtures. Never commit personal ChatGPT exports or real private conversations to a public repository.

## Cross-repository change rules

Use a workspace changeset when changing:

- AI archive/event contracts;
- upload APIs used by Platform/export-agent/web/mobile;
- normalized content consumed by Knowledge;
- BlobStore/asset references;
- completeness/status semantics shown by clients;
- Compliance authentication/deployment;
- migration/backfill/portable export formats.

List producer/consumer compatibility, rollout, rollback, reprocessing/reindexing, storage, privacy, and user-visible completeness impact.

## Git and PR workflow

- State affected acquisition mode and provider schema/parser versions.
- Keep parser semantic changes separate from unrelated infrastructure refactors.
- Include safe fixtures and completeness/graph evidence.
- Document raw storage, reprocessing, retention, and asset impact.
- Do not add browser login/session automation.
- Do not commit provider credentials, personal exports, raw chats, files, or titles.
- Do not claim completeness without explicit evidence.
- Update README/ADRs when provider format, acquisition capability, or archive semantics change.

## Completion criteria

A task is complete only when:

- responsibility belongs to the ChatGPT Archive context;
- raw provider evidence is stored immutably and verified;
- archive intake is hostile-input safe;
- parser/schema versions and unknown records are preserved;
- projects, graph conversations, content parts, files, and assets reconcile idempotently;
- completeness is explicit and conservative;
- absence from one snapshot does not cause deletion;
- no browser-session automation or inference responsibility is introduced;
- portable output and downstream events preserve provenance;
- relevant security/import/graph/export tests pass;
- contracts, migrations, telemetry, privacy, and cross-repository rollout are documented.
