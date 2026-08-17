# Developing Ratatoskr ChatGPT Archive

> Status: Proposed  
> Last reviewed: 2026-08-17

Architecture bootstrap: importer, parser registry, schema, Compliance adapter, storage, and portable exporter are not implemented.

## Intended toolchain

Rust/Tokio, safe ZIP/archive handling, streaming SHA-256, SQLx/PostgreSQL, content-addressed BlobStore, Serde/JSON Schema, NATS, fixture-driven parsers, tracing, and testcontainers.

## Workflow

1. Treat every export as hostile and persist the immutable raw archive before parsing.
2. Detect format/schema and choose an explicit versioned parser.
3. Preserve unknown records/content parts and produce a completeness report.
4. Reconcile projects, conversation graphs, revisions, files, Canvas/assets without overwriting history.
5. Test limits, traversal, duplicates, partial assets, interruption, privacy deletion, and portable export.

The first scaffold PR must document exact commands. Default tests use synthetic export fixtures and never ChatGPT login/session cookies or personal exports.
