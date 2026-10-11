## Context

Review finding (major): with two exports of one conversation where the second carries edited text, deleting the second left the edited text in `content_parts` and queued a `conversation.updated` fact containing it after the tombstone. The same holds for an edited project title or description. XR-021 CONTRACTS.md R3-06 requires one row per provider id; S07 requires that deleting an export removes what only that export held. Both hold only if the row can be put back to a version a retained export presented.

## Decisions

- Ownership is by version, not by row. Each observation names the digest of the version the export presented (`export_entity_observations.content_digest`). `entity_versions(entity_kind, entity_id, content_digest, snapshot)` keeps one snapshot per distinct digest: a `ProjectVersion` (title, description, instructions text, parser stamp) or a `ConversationVersion` (the parsed conversation, its project's provider id, parser stamp). A version exists exactly while some observation names it: `observe` drops the version an export stops presenting when no other export presents it, and a deletion drops the rest. Storing the snapshot only when the content first appears keeps the cost proportional to edits, not to exports.
- A record is stale after a deletion exactly when it keeps a version that no retained observation names. That test needs no knowledge of which export wrote what, and it also catches an intermediate version between two retained ones, which a "latest fact names a deleted archive" test would miss.
- Restore is replay. A project holds one state, so it takes the newest retained version (retained observers ordered by `exports.received_at`, then id). A conversation's content parts are an append-only history that no export owns, so they are deleted and written again by the importer's own writer from the retained versions in export order; message rows keep their ids because messages are upserted by provider id. The header ends as the newest retained export left it. The latest revision is aligned with the restored digest so the history stays consistent.
- The rows phase restores before it queues the tombstones' repair facts, in the same transaction, so a survivor fact is built from restored rows. The fault seam `AfterTombstones` still rolls back the tombstones, the restore and the repair together.
- The provenance of a survivor fact is the earliest retained export that presented the current content, with the earlier rule (earliest retained observer) as the fallback when no retained export carries a digest. An export that presented older content is not evidence of the content in the fact.
- An export that reads a conversation unchanged observes the messages it presented. Observing every message of the shared row made a later export evidence of a message only an earlier version held, which kept that message after the earlier export was deleted.
- The reparse comparison reads the export's own version digest. The revision history of the shared row describes the record, not any one export.
- Not logged: the Platform operation id in a refused-receipt line. The work order for this round asks for no header value in those lines and its test pins that; CONTRACTS.md R3-07 rule 2 asks for the operation or run id. The stricter privacy rule is kept and the conflict is reported for the owner to resolve in the contract.

## Risks

- A reparse of an older export that changes its content writes that content to the shared row, which is then older than another export's version. The versions and the restore keep the deletion correct, but the row is not re-ordered by export age. Not changed here.
- Existing development databases must be recreated from the edited `schema.sql`; there are no migrations.
