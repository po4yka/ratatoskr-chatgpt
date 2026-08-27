# archive-reconciliation Specification

## Purpose

Reconciles ordered, parsed archive snapshots into durable identity, revision,
and graph evidence without treating an omitted record as a deletion.

## Requirements

### Requirement: Repeated conversation evidence forms append-only revision chains

For each ordered archive snapshot, the service SHALL identify a conversation
and each of its messages by its provider external ID within the reconciliation
scope. It SHALL calculate a deterministic digest from the complete normalized
record evidence. A new digest for an existing identity SHALL append a new
revision, while an already observed digest SHALL reuse the existing revision;
neither outcome SHALL overwrite historical revisions or create a duplicate
revision for identical evidence.

#### Scenario: changed snapshot appends only changed identities

- **WHEN** `revision_chain_builds_across_fixture_exports` reconciles two
  fixture exports where one stable conversation and message have changed while
  another remains byte-for-byte equivalent
- **THEN** the changed conversation and message have two ordered revisions,
  the unchanged identities retain one revision each, and every archive
  observation points to its matching revision

### Requirement: Missing snapshot records remain non-destructive observations

When a conversation, project, instruction, Canvas document, or asset observed in
an earlier ConsumerExport snapshot is absent from a later snapshot, the service
SHALL add a `MissingFromLatestSnapshot` observation for that archive. It SHALL
retain the record, all of its earlier revisions, and its last present
observation, and it SHALL NOT emit or infer an explicit provider-deletion state,
lost access, or byte availability.

#### Scenario: later omission records absence without erasing evidence

- **WHEN** `missing_conversation_becomes_observation_not_deletion` reconciles
  a fixture sequence whose second export omits a previously observed
  conversation
- **THEN** the result records one missing observation for that archive and
  still exposes the conversation's earlier revision and present observation

#### Scenario: later project omission remains non-destructive

- **WHEN** `missing_project_evidence_is_an_observation_not_a_deletion`
  reconciles a fixture sequence whose second export omits a previously observed
  project and instruction
- **THEN** the result retains their earlier revisions and records a missing
  observation without fabricating a deletion or changing asset availability

### Requirement: Conversation graphs retain and warn on inconsistent parents

For every parsed conversation, the service SHALL validate that a message parent
is present in the same conversation and is not the message itself. A message
with a missing, cross-conversation, or self parent SHALL remain in the
reconciled conversation as an explicit orphan and SHALL generate a structured
graph warning. The service SHALL NOT drop the message, silently reparent it,
or use it to infer a provider deletion.

#### Scenario: orphan message survives graph validation

- **WHEN** `orphan_parent_is_retained_and_reported` reconciles a fixture
  conversation containing a message whose parent ID is absent from that
  conversation
- **THEN** the message is present in reconciled evidence as an orphan and the
  archive warning identifies the missing-parent condition without exposing
  message content

### Requirement: Unobserved project relationships remain explicit gaps

When a supported parser supplies conversations but no project membership
evidence, the service SHALL preserve the conversations and record project
relationship coverage as unobserved. It SHALL NOT invent a project, attach a
conversation to a guessed project, or classify the absent project relationship
as a conversation deletion.

#### Scenario: conversation-only export does not invent a project

- **WHEN** `conversation_only_snapshot_reports_project_relationship_gap`
  reconciles a synthetic export with no project records or membership fields
- **THEN** the cumulative result retains the conversation and exposes an
  explicit project-relationship coverage gap
