# ChatGPT archive implementation plan

1. Scaffold service, config, telemetry, health, errors, BlobStore, and `chatgpt_archive` schema.
2. Implement authenticated archive receipt, streaming hash, immutable raw storage, and durable import state.
3. Implement safe inspector/extractor and parser registry.
4. Add first synthetic schema parser for conversations/messages/content parts.
5. Add graph/revision reconciliation and completeness report.
6. Add files/assets/Canvas and project metadata/instructions where evidenced.
7. Publish normalized events and integrate with Knowledge/search.
8. Implement deterministic portable JSON/Markdown/assets export.
9. Add privacy deletion, reparse, parser-version migration, and real owner-provided fixture discovery process.
10. Add optional Compliance adapter as a separate acquisition mode.

Definition of Done: raw evidence survives failures, parser is safe/versioned/loss-aware, graph and completeness validate, portable export works, privacy/migrations/events/tests and workspace flow pass.
