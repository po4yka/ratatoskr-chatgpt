# Ratatoskr ChatGPT

`ratatoskr-chatgpt` is the ChatGPT archive bounded context for Ratatoskr. It preserves official ChatGPT data exports as immutable evidence, normalizes projects and conversation graphs, archives referenced assets, and publishes searchable local projections without depending on a live ChatGPT browser session.

> **Status:** service scaffold and authenticated archive receipt implemented. The Rust workspace builds a `ratatoskr-chatgpt-archive` binary that boots with typed configuration, structured JSON telemetry, `/health/live` + `/health/ready` + `/metrics` + `/version` endpoints, content-addressed BlobStore storage, applies the first-version `chatgpt_archive` schema (`schema.sql`), and serves `POST /exports`: an authenticated, tenant-scoped receipt that streams uploads through SHA-256 into isolated staging, enforces the configured size cap, publishes immutable raw evidence through BlobStore, records durable resumable import runs (`received -> hashed -> stored -> inspected -> parsed -> reconciled -> complete/partial`, `failed`/`duplicate` terminals), and answers duplicate archives explicitly by per-tenant digest. Not implemented yet: safe archive inspection/extraction and parsers (plan items 3–4), graph reconciliation and completeness reports (item 5), events, portable exports, Compliance adapters.

> [!IMPORTANT]
> **Ratatoskr is in development.** No database holds data that has to survive a schema change.
> While this status holds, these two rules replace what the documents below plan:
>
> - the API and the database keep their first version. There is no `v2` and no later major
>   version.
> - the database has no migrations. The first-version definition lives in `schema.sql`, is
>   embedded into the binary, and later schema changes edit that file in place.
>
> Only the repository owner changes this status.

## Running locally

```bash
docker compose up -d postgres          # PostgreSQL 17 on 127.0.0.1:5439
export RATATOSKR__ADMIN__LISTEN_ADDRESS=127.0.0.1:9084
export RATATOSKR__STORAGE__BLOB_ROOT=/tmp/ratatoskr-chatgpt/blobs
export RATATOSKR__STORAGE__RECEIPT_STAGING_ROOT=/tmp/ratatoskr-chatgpt/staging
export RATATOSKR__RECEIPT__TENANT_TOKENS='dev-token=local-account'
export RATATOSKR__STORAGE__DATABASE_URL=postgres://chatgpt:chatgpt@127.0.0.1:5439/chatgpt
cargo run -p ratatoskr-chatgpt-archive-service
curl http://127.0.0.1:9084/health/ready
```

With staging root and tokens configured, upload an export:

```bash
curl -X POST http://127.0.0.1:9084/exports \
  -H 'Authorization: Bearer dev-token' \
  -H 'X-Ratatoskr-Acquisition: consumer_export' \
  -H 'Content-Type: application/zip' \
  --data-binary @chatgpt-export.zip
```

The validation commands are in `DEVELOPMENT.md`; CI enforces the same list.

## Product boundary

This repository archives the **ChatGPT product** and its user-visible data. It is not an OpenAI API client and does not treat OpenAI Responses/Conversations API state as equivalent to ChatGPT history.

It owns:

- ChatGPT account/archive identity;
- raw official export snapshots;
- projects and project metadata discovered in exports;
- conversation graphs and message revisions;
- uploaded and generated files;
- Canvas and other discovered artifacts;
- citations and source references;
- provider-specific raw JSON and unknown record variants;
- completeness reports;
- optional Enterprise/Edu Compliance ingestion adapters;
- portable local Markdown/JSON project exports.

It does not call GPT models, store ChatGPT passwords or cookies, automate the consumer web interface, or own semantic search.

## Acquisition modes

```rust
pub enum ChatGptAcquisition {
    ConsumerExport,
    EduExport,
    ComplianceLog,
    ManualConversationCapture,
    LegacyImport,
}
```

### Consumer export

For personal ChatGPT accounts the supported workflow is snapshot-based:

```text
ChatGPT Settings or Privacy Portal
  -> request official export
  -> download ZIP from the provider email
  -> ratatoskr-export-agent
  -> immutable BlobStore
  -> ratatoskr-chatgpt importer
```

This is a periodic local backup, not a continuous upstream replica.

### Enterprise and Edu

A separately configured adapter may consume available Compliance logs or organization exports. Continuous log ingestion and periodic full exports are complementary:

- Compliance events provide incremental activity within provider retention limits.
- Full organization exports reconcile structure and assets.
- The service records the actual capabilities of the configured workspace instead of assuming that every project field is available.

## Raw-first archive model

The original provider archive is always stored before normalization:

```text
/raw/chatgpt/YYYY/MM/<sha256>.zip
```

The raw ZIP, detected schema, parser version, import warnings, and resulting normalized records form one immutable `ProviderExport` snapshot.

Normalization never replaces the provider evidence. Parser upgrades can replay old exports from the preserved archive without requesting the data again.

## Planned data model

The service will own a `chatgpt_archive.*` PostgreSQL schema when persistence is implemented:

```text
chatgpt_accounts
chatgpt_exports
chatgpt_import_runs
chatgpt_projects
chatgpt_project_revisions
chatgpt_project_sources
chatgpt_conversations
chatgpt_conversation_revisions
chatgpt_messages
chatgpt_message_revisions
chatgpt_content_parts
chatgpt_attachments
chatgpt_artifacts
chatgpt_citations
chatgpt_raw_records
chatgpt_completeness_reports
chatgpt_tombstones
outbox_events
inbox_events
```

Large archives, attachments, generated assets, Canvas payloads, raw JSON, and portable exports use the content-addressed BlobStore.

## Projects

A ChatGPT project may contain more than conversations. The importer preserves every discovered component independently:

- project identity and title;
- description;
- project instructions;
- conversations;
- uploaded files and pasted sources;
- saved responses;
- generated files;
- Canvas or similar artifacts;
- external references;
- sharing or membership metadata when present;
- first and last observed export snapshots.

Consumer export documentation does not guarantee that every internal project property is reconstructable. The service therefore reports discovered coverage and never labels an import complete solely because conversation JSON parsed successfully.

External references are modeled explicitly:

```text
provider_reference
original_url
original_provider
observed_title
observed_version
locally_backed_up
local_blob_ref
```

A Google Drive, Slack, or GitHub reference is not considered locally preserved until the owning connector has archived the referenced content.

## Conversations are graphs

A conversation is not assumed to be one linear list. The normalized model supports:

- edits to earlier user messages;
- regenerated assistant responses;
- branches;
- hidden, system, and tool messages when present;
- interrupted or incomplete responses;
- citations;
- generated files and tools;
- revision history across exports.

```rust
pub struct ArchivedMessage {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub external_id: Option<String>,
    pub parent_message_id: Option<Uuid>,
    pub role: MessageRole,
    pub content: Vec<ContentPart>,
    pub model: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub provider_metadata: serde_json::Value,
}
```

A provider message ID plus export snapshot identifies an observation. Content changes produce revisions; historical data is not overwritten.

## Content parts

```rust
pub enum ContentPart {
    Text(String),
    Markdown(String),
    Image(BlobRef),
    File(AttachmentRef),
    Code(CodeBlock),
    Citation(Citation),
    ToolCall(ToolCall),
    ToolResult(ToolResult),
    Canvas(CanvasRef),
    GeneratedAsset(AssetRef),
    Unknown(serde_json::Value),
}
```

`Unknown` is mandatory. A new provider record variant must remain recoverable until a parser version understands it.

## Safe import pipeline

```text
1. Receive archive
2. Compute SHA-256
3. Store the original ZIP immutably
4. Inspect archive limits and paths
5. Detect ChatGPT export schema
6. Extract in an isolated temporary directory
7. Preserve original manifests and JSON
8. Parse into staging tables
9. Validate graph and project relationships
10. Store attachments by content hash
11. Reconcile immutable revisions
12. Produce completeness report
13. Publish archive events
14. Queue optional Knowledge indexing
```

Archive safety requirements:

- reject path traversal and absolute paths;
- bound file count and total decompressed size;
- detect suspicious compression ratios and zip bombs;
- sniff content types instead of trusting filenames;
- never execute archive contents;
- never render active HTML during import;
- confine temporary extraction paths;
- preserve malformed or unknown records with warnings rather than silently dropping them.

## Snapshot and deletion semantics

Every official export remains an independent snapshot:

```text
snapshot-2026-08
snapshot-2026-09
snapshot-2026-10
```

Upstream state is represented conservatively:

```rust
pub enum UpstreamState {
    Present,
    MissingFromLatestSnapshot,
    ExplicitlyDeleted,
    AccessLost,
    Unknown,
}
```

Absence from one export does not prove deletion. A project or conversation may be omitted because of export scope, sharing changes, parser limitations, or provider behavior.

Local hard deletion is governed by an explicit Ratatoskr retention policy and never occurs as an automatic consequence of one incomplete snapshot.

## Completeness reporting

Each import produces a structured report, for example:

```text
Projects discovered:              19
Conversations discovered:        684
Messages discovered:           9,441
Attachments referenced:          227
Attachments archived:            219
Missing attachments:               8
Project instructions found:       14 / 19
Canvas documents found:            11
Unknown record variants:            4
Completeness: STRUCTURALLY_PARTIAL
```

Planned statuses:

```rust
pub enum ExportCompleteness {
    Complete,
    ConversationsComplete,
    StructurallyPartial,
    AssetsPartial,
    Unknown,
    FailedValidation,
}
```

`Complete` requires positive evidence from a known schema and validation rules. It is never the optimistic default.

## Portable local representation

Ratatoskr can generate a provider-independent archive without discarding the raw export:

```text
project-name/
├── project.json
├── README.md
├── instructions.md
├── conversations/
│   ├── chat-001.md
│   ├── chat-001.json
│   └── ...
├── sources/
├── canvas/
├── attachments/
└── manifest.json
```

The portable representation is designed for local reading, versioning, migration, and recovery even when Ratatoskr is unavailable.

## Search and analysis integration

After an accepted import, this service publishes normalized archive events. `ratatoskr-knowledge` owns:

- full-text and semantic indexing;
- cross-provider topic clustering;
- summaries and project digests;
- decision and action-item extraction;
- linking conversations to repositories and documents.

Knowledge references immutable archive source IDs and content hashes. It never replaces or mutates the original conversation evidence.

## Commands and events

Expected contracts include:

```text
chatgpt.export.ingest_requested.v1
chatgpt.export.ingested.v1
chatgpt.export.partial.v1
chatgpt.project.upserted.v1
chatgpt.conversation.upserted.v1
chatgpt.asset.stored.v1
chatgpt.snapshot.completed.v1
chatgpt.compliance.sync_requested.v1
chatgpt.compliance.cursor_advanced.v1
ai_archive.project.changed.v1
ai_archive.conversation.changed.v1
```

Import and event handlers are idempotent. The archive SHA-256 prevents duplicate provider exports; record identities and revisions prevent duplicate normalized rows.

## Security and privacy invariants

1. The original export is immutable and hash-addressed.
2. No ChatGPT password, session cookie, or undocumented browser token is collected.
3. Archive contents are untrusted and safely extracted.
4. Provider-specific unknown data is preserved losslessly.
5. Tenant ownership is enforced on every project, conversation, and attachment.
6. Raw exports and sensitive conversations are never emitted into logs or traces.
7. External model analysis is opt-in and governed by Knowledge provider policy.
8. Temporary or incognito content is not claimed as backed up unless present in an authoritative export or Compliance source.
9. Missing records do not trigger destructive local deletion.

Receipt authentication today is a configured bearer-token map (`RATATOSKR__RECEIPT__TENANT_TOKENS`), one token per personal-kind tenant account. This is an explicit stopgap until Platform issues device tokens: there is no rotation, no per-token audit identity beyond the account reference, and workspace-kind accounts cannot be expressed yet. Treat the token list like any other secret-bearing configuration.

## Observability

Core metrics include:

```text
chatgpt_export_import_duration
chatgpt_export_bytes
chatgpt_projects_imported
chatgpt_conversations_imported
chatgpt_messages_imported
chatgpt_missing_assets
chatgpt_unknown_record_variants
chatgpt_completeness_status
chatgpt_import_failures
chatgpt_compliance_cursor_age
chatgpt_snapshot_age
```

Every import records archive hash, acquisition method, detected schema, parser version, counts, warnings, completeness, and operation correlation.

## Non-goals

- Calling OpenAI models or implementing an inference gateway.
- Treating OpenAI API Conversation state as ChatGPT product history.
- Browser automation of the ChatGPT consumer interface.
- Storing user passwords, cookies, or MFA secrets.
- Claiming a live replica for personal-account exports.
- Treating one parsed conversations file as proof of complete project backup.
- Owning semantic search or derived analysis.
- Replacing raw provider evidence with a portable Markdown export.

## Initial milestones

1. Define export, project, conversation-graph, message, asset, and completeness schemas.
2. Implement safe immutable archive ingestion.
3. Discover and version the first real personal ChatGPT export schema.
4. Import conversations and preserve branching/revisions.
5. Import projects, files, Canvas, citations, and unknown records where present.
6. Produce completeness reports and portable exports.
7. Integrate the macOS export agent.
8. Publish normalized events for Knowledge indexing.
9. Add Enterprise/Edu Compliance adapters behind explicit configuration.
10. Add replay tests across multiple historical export fixtures.

## Workspace integration

Planned: `ratatoskr-workspace` will pin this repository with compatible AI-archive contracts, Export Agent, Platform, Knowledge, Web, and Mobile commits. No workspace pin or integration profile exists for this service today. Real user exports used as fixtures must be sanitized or kept in protected test storage; public CI will use synthetic archives.

## Project status

This README defines the intended ChatGPT archive architecture. The service scaffold (configuration, telemetry, health endpoints, errors, BlobStore, first-version schema) is implemented and tested; no importer, schema parser, Compliance connector, or portable export generator exists yet.
