## MODIFIED Requirements

### Requirement: Finalization couples database erasure, audit, and downstream tombstones

A deletion SHALL become terminally completed only after every exclusively owned blob in its inventory is absent and every shared blob is proven reachable from the rows that remain. The row-removal transaction SHALL remove the selected normalized and provenance records and enqueue one authoritative `ai_archive.subject.tombstoned.v1` outbox record with `reason = "user_requested"` for each downstream subject that no longer has retained evidence. The completion transaction SHALL append an immutable content-free audit outcome and drop the request items. If either transaction fails, none of its database effects SHALL commit and the durable request SHALL remain resumable.

#### Scenario: Final transaction failure is atomic

- **WHEN** persistence fails while the rows of a deletion are being removed or while it is being completed
- **THEN** normalized row removal, completion audit, and Knowledge tombstone outbox insertion are all absent, the request remains `planned`, and a retry can run from the recorded inventory

#### Scenario: Completed deletion has matching audit and outbox evidence

- **WHEN** a deletion completes
- **THEN** its completion audit category counts match the executed inventory and every no-longer-evidenced downstream subject has exactly one replay-safe tombstone outbox record

## ADDED Requirements

### Requirement: Deletion removes rows first, bytes second, and completes last

Deletion SHALL delete the scope rows and their dependents, repoint retained provenance and record the rows-removed instant in one transaction under the tenant lock, then erase bytes, then write the audit and complete the request. At no commit point SHALL a surviving row reference a missing blob, and every blob not yet erased SHALL be listed in a durable item. A crash at any point SHALL converge on re-execution.

#### Scenario: Crash between the row phase and the byte phase

- **WHEN** execution stops after the rows are committed
- **THEN** the scope rows are gone, the request is `purging`, the raw blob still exists, and re-execution erases it and completes the request

#### Scenario: Completed request replays its report

- **WHEN** a completed request is executed again
- **THEN** the stored report is returned unchanged

### Requirement: The deletion closure covers every foreign key into a deletion root

Deletion SHALL remove `platform_operation_imports` and `reparse_runs` of the selected exports, and SHALL repoint retained projects, conversations and assets whose first-seen or observed export is deleted to the earliest retained observing export, or to NULL. A test SHALL fail when a foreign key into a deletion root is neither cascading nor handled.

#### Scenario: Archive deletion with a Platform operation and a reparse run

- **WHEN** an archive that has a `platform_operation_imports` row and a `reparse_runs` row is deleted
- **THEN** the request completes and both tables hold no row for it

### Requirement: Tombstones are complete envelopes owned by the Platform user

A user-requested deletion SHALL enqueue one `ai_archive.subject.tombstoned.v1` envelope per deleted archive, conversation and project, owned by the mapped Platform user, with reason `user_requested`. An account without a mapping SHALL emit none and the completion totals SHALL report `downstream_tombstone_unbound`. A tenant deletion SHALL keep unpublished tombstones of earlier requests.

#### Scenario: Archive deletion tombstone

- **WHEN** an archive of a mapped account is deleted
- **THEN** the outbox row decodes as an `EventEnvelope` whose tenant and payload owner are `user:<Platform user>`, and whose evidence blob exists
