## Decisions

- One function owns every removal of a published content-bearing fact row: `DELETE ... RETURNING` on the outbox rows selected by inventory id or by `export_id`. Returning the owner (envelope tenant) and `published_stream_seq` from the delete itself means the sequences are exactly the rows removed, with no gap between collecting and deleting. The export cascade then finds nothing left to cascade.
- Only the seven content-bearing fact types count; operation reports, tombstones and erasure commands are never listed.
- Commands are grouped by owner and chunked by 500 in ascending order, with a deduplication key `stream-erasure:<request>:<owner>:<chunk>` so a re-run of phase A can never mint a second command.
- The command is a `CommandEnvelope` with `command_id` minted when the row is built (UUIDv7). The pump learns the command type through the closed type-to-subject function and relays the stored document by its `command_id`.
- Phase A does not wait for Edge. The erasure is eventual, like the tombstone publication.
- The command row has no export and is bound to the account, so the tenant-scope removal keeps an unpublished one.
