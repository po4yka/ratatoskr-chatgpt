## MODIFIED Requirements

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
