## Why

The independent review of XR-021 round 3 rejected the survivor repair of privacy deletion (CONTRACTS.md R3-06): a project or conversation is now one shared row for every cumulative export, and that row, the append-only content-part history under it and the fact built from it carry the newest export's content. Deleting the export that wrote that content kept it, because an older export still observes the record, and the repair fact then sent the deleted export's text to Knowledge under the retained export's provenance after the tombstone. Before the shared row each export owned its own copy, so deleting it removed its version completely. The privacy-deletion contract (S07) says a deleted export's content is gone, so this is a defect, not a design choice.

## What Changes

- Every export records, per project and conversation it observes, the digest of the version it presented, and one snapshot per distinct version is kept while some export names it (`export_entity_observations.content_digest`, `entity_versions`, `projects.content_digest`, `conversations.content_digest`).
- The row-removal transaction of a privacy deletion rebuilds every surviving project and conversation that keeps a version no retained export presented: a project takes the newest retained version, a conversation is rewritten from the retained versions in export order, so the row, its content-part history, its revision history and its snapshots carry only what a retained export holds. This runs before the survivors are announced again.
- A survivor fact names the earliest retained export that presented the content the record holds now, not merely the earliest export that observed the record.
- An export that reads a conversation unchanged observes only the messages it presented, not every message the shared row has accumulated.
- The reparse comparison reads the export's own version digest instead of the latest revision of the shared row.

Breaking, with no shims: `schema.sql` gains `entity_versions` and three `content_digest` columns in place (a development database must be recreated); `ParsedConversation`, `ParsedMessage`, `ParsedContentPart`, `MessageRole` and `ContentPartKind` derive `Serialize` and `Deserialize`.

## Impact

Touches the importer's persistence, reparse planning, privacy deletion (rows phase and survivor facts), `schema.sql` and the README. The explicit decision not to log the Platform operation id in refused-receipt lines (the work order forbids header values in those lines; CONTRACTS.md R3-07 rule 2 asks for the id) is recorded in the design and reported, not changed.
