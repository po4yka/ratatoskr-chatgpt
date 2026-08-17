# ChatGPT archive data model

## Owned schema: `chatgpt_archive.*`

- `accounts`, acquisition connections and optional encrypted Compliance credentials.
- `exports`: archive hash/blob, acquisition, received/imported time, detected schema, parser, completeness.
- `import_runs`, staged record summaries, warnings, unknown records/blob refs.
- `projects`, project revisions, instructions, sources/references, memberships where available.
- `conversations`, branches, messages, message revisions, content parts, citations/tool records.
- `assets`, file/media metadata, blob refs, Canvas/generation relations.
- snapshot observations, upstream/access states, portable exports, outbox/inbox.

## Constraints

Archive hash is immutable/idempotent. Provider IDs are scoped and stable when present; deterministic surrogate identities record derivation. Parent/revision graph integrity is validated without discarding orphans. Blob hashes are unique and owner-authorized. Completeness is evidence, not a boolean promise. Cross-schema writes/foreign keys are forbidden.
