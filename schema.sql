-- The first version of the chatgpt_archive schema, owned by ratatoskr-chatgpt.
--
-- Development status: one version only, no migrations. This file is edited in
-- place while no database has to survive a change; `apply_schema` embeds it
-- into the binary and runs it inside one advisory-locked transaction.
--
-- Rules this file obeys (AGENTS.md):
-- - every statement is idempotent, so application is repeatable;
-- - normalized projections stay separable from raw evidence (`raw_records`);
-- - graph relationships are first-class (parent messages, relations);
-- - blob bytes are referenced by fleet BlobRef JSON, never stored inline;
-- - completeness is explicit and conservative.

CREATE SCHEMA IF NOT EXISTS chatgpt_archive;

-- The ChatGPT account or workspace an archive belongs to.
CREATE TABLE IF NOT EXISTS chatgpt_archive.accounts (
    id              UUID PRIMARY KEY,
    external_kind   TEXT NOT NULL CHECK (external_kind IN ('personal', 'team_workspace', 'enterprise_workspace', 'edu_workspace')),
    external_ref    TEXT,
    display_label   TEXT,
    first_seen_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT accounts_external_ref_unique UNIQUE (external_kind, external_ref)
);

-- One received provider archive or compliance feed. The immutable original
-- lives in BlobStore; this row carries its reference and provenance.
-- Digest uniqueness is per tenant: two accounts may hold equal bytes as
-- separate exports while BlobStore deduplicates storage by content address.
CREATE TABLE IF NOT EXISTS chatgpt_archive.exports (
    id                UUID PRIMARY KEY,
    ai_archive_id     UUID NOT NULL,
    account_id        UUID NOT NULL REFERENCES chatgpt_archive.accounts (id),
    acquisition_mode  TEXT NOT NULL CHECK (acquisition_mode IN ('consumer_export', 'edu_export', 'compliance_log', 'manual_capture', 'legacy_import')),
    blob_ref          JSONB NOT NULL,
    sha256_hex        CHAR(64) NOT NULL,
    byte_length       BIGINT NOT NULL CHECK (byte_length >= 0),
    received_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    import_started_at TIMESTAMPTZ,
    CONSTRAINT exports_digest_per_account_unique UNIQUE (account_id, sha256_hex)
);

-- Durable state machine runs for one export. A run exists before its export
-- row materializes (nullable link); digest evidence lands at `hashed`.
-- The parser version stays unset until a parser has touched the run.
CREATE TABLE IF NOT EXISTS chatgpt_archive.import_runs (
    id              UUID PRIMARY KEY,
    export_id       UUID REFERENCES chatgpt_archive.exports (id),
    account_ref     TEXT,
    acquisition_mode TEXT,
    media_type      TEXT,
    parser_version  TEXT,
    schema_id       TEXT,
    state           TEXT NOT NULL CHECK (state IN ('received', 'hashed', 'stored', 'inspected', 'parsed', 'reconciled', 'completed', 'partial', 'failed', 'duplicate', 'quarantined')),
    correlation_id  UUID,
    sha256_hex      CHAR(64),
    byte_length     BIGINT CHECK (byte_length IS NULL OR byte_length >= 0),
    started_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at     TIMESTAMPTZ
);

-- Projects observed in exports; components arrive as later columns/tables, in place.
CREATE TABLE IF NOT EXISTS chatgpt_archive.projects (
    id                 UUID PRIMARY KEY,
    account_id         UUID REFERENCES chatgpt_archive.accounts (id),
    external_id        TEXT NOT NULL,
    title              TEXT,
    description        TEXT,
    instructions       TEXT,
    archived_observed  BOOLEAN NOT NULL DEFAULT FALSE,
    first_seen_export  UUID REFERENCES chatgpt_archive.exports (id),
    last_seen_export   UUID REFERENCES chatgpt_archive.exports (id),
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (account_id, external_id)
);

-- Conversations are graphs, not lists: parent links live on messages.
CREATE TABLE IF NOT EXISTS chatgpt_archive.conversations (
    id                 UUID PRIMARY KEY,
    project_id         UUID REFERENCES chatgpt_archive.projects (id),
    account_id         UUID REFERENCES chatgpt_archive.accounts (id),
    external_id        TEXT NOT NULL,
    title              TEXT,
    conversation_kind  TEXT NOT NULL DEFAULT 'standard' CHECK (conversation_kind IN ('standard', 'temporary', 'branch', 'unknown')),
    first_seen_export  UUID REFERENCES chatgpt_archive.exports (id),
    last_seen_export   UUID REFERENCES chatgpt_archive.exports (id),
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS chatgpt_archive.messages (
    id                 UUID PRIMARY KEY,
    conversation_id    UUID NOT NULL REFERENCES chatgpt_archive.conversations (id),
    external_id        TEXT,
    parent_message_id  UUID REFERENCES chatgpt_archive.messages (id),
    role               TEXT NOT NULL CHECK (role IN ('system', 'user', 'assistant', 'tool', 'internal', 'unknown')),
    model_slug         TEXT,
    generation_index   INTEGER,
    interrupted        BOOLEAN NOT NULL DEFAULT FALSE,
    created_at         TIMESTAMPTZ,
    updated_at         TIMESTAMPTZ,
    provider_metadata  JSONB NOT NULL DEFAULT '{}'::jsonb,
    UNIQUE (conversation_id, external_id)
);

-- Edits and regenerated responses: one message may carry several content
-- revisions across exports.
CREATE TABLE IF NOT EXISTS chatgpt_archive.message_relations (
    id                  UUID PRIMARY KEY,
    from_message_id     UUID NOT NULL REFERENCES chatgpt_archive.messages (id),
    to_message_id       UUID NOT NULL REFERENCES chatgpt_archive.messages (id),
    relation_kind       TEXT NOT NULL CHECK (relation_kind IN ('edit_of', 'regeneration_of', 'continues', 'cites', 'tool_output_of', 'unknown')),
    observed_in_export  UUID REFERENCES chatgpt_archive.exports (id),
    UNIQUE (from_message_id, to_message_id, relation_kind)
);

-- Typed heterogeneous content parts with their position inside one revision
-- of a message. Unknown variants land here too, never discarded.
CREATE TABLE IF NOT EXISTS chatgpt_archive.content_parts (
    id               UUID PRIMARY KEY,
    message_id       UUID NOT NULL REFERENCES chatgpt_archive.messages (id),
    revision         INTEGER NOT NULL DEFAULT 0,
    ordinal          INTEGER NOT NULL,
    part_kind        TEXT NOT NULL CHECK (part_kind IN ('text', 'markdown', 'image', 'file', 'code', 'citation', 'tool_call', 'tool_result', 'artifact', 'canvas', 'unknown')),
    payload          JSONB NOT NULL,
    blob_ref         JSONB,
    UNIQUE (message_id, revision, ordinal)
);

-- Uploaded and generated files whose bytes may or may not be archived.
CREATE TABLE IF NOT EXISTS chatgpt_archive.assets (
    id               UUID PRIMARY KEY,
    external_id      TEXT NOT NULL,
    asset_kind       TEXT NOT NULL DEFAULT 'unknown' CHECK (asset_kind IN ('uploaded_file', 'generated_image', 'generated_file', 'canvas_document', 'unknown')),
    provider_name    TEXT,
    media_type       TEXT,
    byte_length      BIGINT CHECK (byte_length IS NULL OR byte_length >= 0),
    sha256_hex       CHAR(64),
    blob_ref         JSONB,
    locally_backed_up BOOLEAN NOT NULL DEFAULT FALSE,
    observed_in      UUID REFERENCES chatgpt_archive.exports (id)
);

-- Revisions of any normalized entity across repeated snapshots.
CREATE TABLE IF NOT EXISTS chatgpt_archive.revisions (
    id                UUID PRIMARY KEY,
    entity_table      TEXT NOT NULL,
    entity_id         UUID NOT NULL,
    revision_number   INTEGER NOT NULL,
    observed_in       UUID NOT NULL REFERENCES chatgpt_archive.exports (id),
    payload           JSONB NOT NULL,
    recorded_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (entity_table, entity_id, revision_number)
);

-- Provider records the current parser does not understand yet, preserved
-- losslessly for reprocessing under a newer parser version.
CREATE TABLE IF NOT EXISTS chatgpt_archive.raw_records (
    id           UUID PRIMARY KEY,
    export_id    UUID NOT NULL REFERENCES chatgpt_archive.exports (id),
    record_path  TEXT NOT NULL,
    record_kind  TEXT NOT NULL DEFAULT 'unknown',
    payload      JSONB NOT NULL,
    recorded_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- What was and was not present in one import run.
CREATE TABLE IF NOT EXISTS chatgpt_archive.completeness_reports (
    id                UUID PRIMARY KEY,
    import_run_id     UUID NOT NULL REFERENCES chatgpt_archive.import_runs (id),
    status            TEXT NOT NULL CHECK (status IN ('complete', 'conversations_complete', 'structurally_partial', 'assets_partial', 'unknown', 'failed_validation')),
    counts            JSONB NOT NULL DEFAULT '{}'::jsonb,
    warnings          JSONB NOT NULL DEFAULT '[]'::jsonb,
    missing_assets    INTEGER NOT NULL DEFAULT 0,
    unknown_variants  INTEGER NOT NULL DEFAULT 0,
    produced_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Explicit deletion evidence only. Absence from a snapshot never lands here.
CREATE TABLE IF NOT EXISTS chatgpt_archive.tombstones (
    id                UUID PRIMARY KEY,
    entity_table      TEXT NOT NULL,
    external_id       TEXT NOT NULL,
    reason            TEXT NOT NULL CHECK (reason IN ('provider_deletion_event', 'compliance_event', 'reconciliation_policy', 'access_lost')),
    evidence_ref      TEXT NOT NULL,
    recorded_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Transactional outbox: archive events awaiting publication.
CREATE TABLE IF NOT EXISTS chatgpt_archive.outbox_events (
    id              BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    event_type      TEXT NOT NULL,
    aggregate_id    UUID NOT NULL,
    payload         JSONB NOT NULL,
    correlation_id  UUID,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    published_at    TIMESTAMPTZ
);

CREATE UNIQUE INDEX IF NOT EXISTS outbox_operation_report_once
    ON chatgpt_archive.outbox_events (event_type, aggregate_id)
    WHERE event_type = 'platform.operation.reported.v1';

-- Inbox deduplication: event identity seen from other bounded contexts.
CREATE TABLE IF NOT EXISTS chatgpt_archive.inbox_events (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    source        TEXT NOT NULL,
    event_type    TEXT NOT NULL,
    event_key     TEXT NOT NULL,
    payload       JSONB NOT NULL,
    received_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    processed_at  TIMESTAMPTZ,
    UNIQUE (source, event_type, event_key)
);
