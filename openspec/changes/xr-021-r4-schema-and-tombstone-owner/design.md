## Decisions

- The announced facts are the source of the owner because they are what Knowledge holds. The mapping is operator configuration that can change after the import; the outbox rows are the record of what was told to Knowledge. Reading them is a query inside the row-removal transaction, before `delete_scoped_rows` removes the rows that cascade from the deleted exports.
- The resolution order is scoped from the specific to the general: the imported head of a deleted archive (archive scope), the first announced fact of a deleted conversation or project (conversation scope), the account's latest imported head, then any announced non-tombstone fact of the account (tenant scope). A fact whose payload carries no owner is skipped, not an error.
- The live mapping stays as the fallback for an account that announced nothing, because the emitters still use `OwnerResolver` and a deletion before any announcement of an otherwise mapped account behaves as before.
- Three outcomes instead of an `Option`: a known owner, nothing announced (and no mapping), and announced-but-unresolvable. Only the last is `downstream_tombstone_unbound`; the second finishes its tombstone items without a row because there is nothing for Knowledge to retract.
- The resolution lives in its own module `privacy_deletion/owner.rs`; `rows.rs` stays under the 850-line cap and only consumes the outcome.
- Schema: no replacement is needed for the removed statements. The `CREATE TABLE` definitions already declare each column, so a database created from the file has them.
