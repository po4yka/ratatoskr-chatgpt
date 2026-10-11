# privacy-deletion Specification

## Purpose
Provides tenant-authorized, evidence-complete privacy erasure whose physical and normalized effects are replay-safe, auditable, and propagated to downstream Knowledge state.

## Requirements

### Requirement: Deletion scope is authorized without existence disclosure

The service SHALL require an authenticated tenant for every deletion and SHALL support exactly archive, conversation, and tenant scopes. An identifier outside the authenticated tenant and an unknown identifier SHALL produce the same public result and SHALL not create a deletion request.

#### Scenario: Cross-tenant archive deletion reveals nothing

- **WHEN** tenant A requests deletion of an archive owned by tenant B
- **THEN** the request returns the same not-found result as an unknown archive and no evidence owned by either tenant changes

### Requirement: Preflight enumerates the complete deletion closure

Before erasing bytes or records, the service SHALL persist a deterministic deletion inventory that enumerates every affected raw archive, extracted or normalized artifact blob, import run, completeness report, raw record, project, conversation, message, relation, content part, asset, revision, inbox/outbox record, downstream tombstone subject, and retained shared blob. The inventory SHALL contain category, opaque identity, and action but no message text, title, filename, raw payload, or external account reference.

#### Scenario: Conversation inventory includes all containing raw evidence

- **WHEN** a conversation occurs in two retained archives and deletion is planned for that conversation
- **THEN** both raw archives and their extracted artifacts appear in the inventory, along with every normalized record that loses its last retained provenance and every independently evidenced record that will remain

#### Scenario: Enumeration counts equal itemized actions

- **WHEN** a deletion inventory is completed
- **THEN** each category total equals the number of itemized actions in that category and no persisted target row or blob reference within the selected closure is absent from the inventory

### Requirement: Scope semantics preserve only independently evidenced data

Archive deletion SHALL remove the selected raw export and data whose last retained provenance is that export. Conversation deletion SHALL remove every raw or derived copy containing the conversation and SHALL remove collateral normalized state that no longer has retained raw provenance. Tenant deletion SHALL remove all archive-owned evidence for that tenant. No scope SHALL remove another tenant's records or a blob still referenced by retained evidence.

#### Scenario: Archive deletion preserves a conversation observed elsewhere

- **WHEN** a conversation is normalized from the selected archive and also from a different retained archive for the same tenant
- **THEN** the selected raw archive and its unique derivatives are erased while the conversation remains linked only to the retained archive

#### Scenario: Tenant deletion leaves another tenant intact

- **WHEN** two tenants reference byte-identical content and one tenant is deleted
- **THEN** the deleted tenant has no remaining records or usable blob references and the other tenant can still verify its retained evidence

### Requirement: Finalization couples database erasure, audit, and downstream tombstones

A deletion SHALL become terminally completed only after every exclusively owned blob in its inventory is absent and every shared blob is proven reachable from the rows that remain. The row-removal transaction SHALL remove the selected normalized and provenance records and enqueue one authoritative `ai_archive.subject.tombstoned.v1` outbox record with `reason = "user_requested"` for each downstream subject that no longer has retained evidence. The completion transaction SHALL append an immutable content-free audit outcome and drop the request items. If either transaction fails, none of its database effects SHALL commit and the durable request SHALL remain resumable.

#### Scenario: Final transaction failure is atomic

- **WHEN** persistence fails while the rows of a deletion are being removed or while it is being completed
- **THEN** normalized row removal, completion audit, and Knowledge tombstone outbox insertion are all absent, the request remains `planned`, and a retry can run from the recorded inventory

#### Scenario: Completed deletion has matching audit and outbox evidence

- **WHEN** a deletion completes
- **THEN** its completion audit category counts match the executed inventory and every no-longer-evidenced downstream subject has exactly one replay-safe tombstone outbox record

### Requirement: Deletion execution is idempotent and diagnosable

Replaying a deletion request SHALL not recreate evidence, duplicate tombstones, or alter unrelated data. A failed or interrupted attempt SHALL retain a content-free state and item outcome sufficient to resume, while a completed request SHALL return its original completion report.

#### Scenario: Completed request replay is a no-op

- **WHEN** the same deletion request is executed after completion
- **THEN** it returns the original report, creates no additional audit or outbox rows, and leaves all retained evidence unchanged

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

### Requirement: Blob erasure and blob registration share one lock

The eraser SHALL hold the digest lock for the reference re-check and the byte erase of each blob in one transaction. A registrar SHALL take the same digest lock before inserting the first row that references a digest and SHALL verify the blob is present. If the blob is missing the registrar SHALL re-store it from the bytes it still holds, and otherwise SHALL fail with a retryable class and register nothing. At no commit point SHALL a row reference a missing blob.

#### Scenario: Import of identical bytes while the eraser holds the lock

- **WHEN** the eraser holds the digest lock after its re-check and another tenant's import registers a reference to the same digest
- **THEN** the import blocks until the eraser commits, finds the blob missing, re-stores it from its staged bytes, and the committed state is a row that references an existing blob

#### Scenario: Import commits first

- **WHEN** the import registers its reference before the eraser takes the lock
- **THEN** the eraser's re-check finds the reference and keeps the blob as `retain_shared`

### Requirement: Deleting an archive re-announces the records a retained archive still holds

The row-removal transaction of a privacy deletion SHALL rebuild every project, conversation and artifact whose action is `retain_evidenced` and whose latest announced fact names a deleted archive, and enqueue an `updated` fact whose provenance names the earliest retained observing export, stamped one millisecond after the archive tombstone. A record whose latest fact already names a retained archive is untouched. The deleted exports' `ai_archive.archive.imported.v1` heads SHALL leave the outbox with their rows. The tombstone and the repair SHALL commit together.

#### Scenario: Oldest of three cumulative exports deleted

- **WHEN** exports E1 (A, B), E2 (A, B, C) and E3 (B, C, D) are published and E1 is deleted
- **THEN** the outbox holds the archive tombstone for E1 and after it `updated` facts for A and B naming E2, and none for a conversation only E1 held

#### Scenario: Every export deleted

- **WHEN** every export of the account is deleted
- **THEN** no survivor fact is announced

#### Scenario: Failure between tombstone and repair

- **WHEN** a failure is injected after the tombstone insert
- **THEN** neither the tombstone nor the repair is present, and re-execution converges

### Requirement: A surviving record holds only content a retained archive presented

After a privacy deletion, every project and conversation that remains SHALL hold only content that a retained export presented, in its row, its content-part and revision history, its stored version snapshots and the fact announced for it. When a record keeps a version that no retained export presented, the row-removal transaction SHALL rebuild it from the versions retained exports presented before announcing it again: a project from the newest retained version, a conversation from the retained versions in export order. The announcing fact SHALL name the earliest retained export that presented the content the record holds.

#### Scenario: Conversation edited by the deleted export

- **WHEN** export E1 holds a conversation and export E2 holds it with edited text and E2 is deleted
- **THEN** neither the database nor any fact queued after the tombstone contains the edited text, and the conversation is announced again with E1's text under E1

#### Scenario: Project edited by the deleted export

- **WHEN** export E2 changed the title and description of a project that E1 also holds and E2 is deleted
- **THEN** the project row, its snapshots and every queued fact carry E1's title and description

#### Scenario: Intermediate version

- **WHEN** exports E1, E2 and E3 hold three different texts of one conversation and E2 is deleted
- **THEN** E2's text is gone, the content-part history holds E1's and E3's text in order, and E3's text stays current

#### Scenario: A retained export repeats the edit

- **WHEN** E1 holds the original, E2 the edit and E3 the same edit, and E2 is deleted
- **THEN** the edit stays current and the survivor fact names E3, not E1

#### Scenario: Tenant deletion

- **WHEN** every export of the tenant is deleted
- **THEN** no version snapshot of the tenant remains
