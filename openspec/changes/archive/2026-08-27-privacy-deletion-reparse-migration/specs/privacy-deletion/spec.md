## Purpose

Provides tenant-authorized, evidence-complete privacy erasure whose physical and normalized effects are replay-safe, auditable, and propagated to downstream Knowledge state.

## ADDED Requirements

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

A deletion SHALL become terminally completed only after every exclusively owned blob in its inventory is absent and every shared blob is freshly proven reachable from retained evidence. The final database transaction SHALL remove the selected normalized and provenance records, append an immutable content-free audit outcome, and enqueue one authoritative `ai_archive.subject.tombstoned.v1` outbox record with `reason = "user_requested"` for each downstream subject that no longer has retained evidence. If that transaction fails, none of those database effects SHALL commit.

#### Scenario: Final transaction failure is atomic

- **WHEN** persistence fails while finalizing a deletion after blob erasure
- **THEN** normalized row removal, completion audit, and Knowledge tombstone outbox insertion are all absent, the durable request remains resumable, and a retry can finish from the recorded inventory

#### Scenario: Completed deletion has matching audit and outbox evidence

- **WHEN** a deletion completes
- **THEN** its completion audit category counts match the executed inventory and every no-longer-evidenced downstream subject has exactly one replay-safe tombstone outbox record

### Requirement: Deletion execution is idempotent and diagnosable

Replaying a deletion request SHALL not recreate evidence, duplicate tombstones, or alter unrelated data. A failed or interrupted attempt SHALL retain a content-free state and item outcome sufficient to resume, while a completed request SHALL return its original completion report.

#### Scenario: Completed request replay is a no-op

- **WHEN** the same deletion request is executed after completion
- **THEN** it returns the original report, creates no additional audit or outbox rows, and leaves all retained evidence unchanged
