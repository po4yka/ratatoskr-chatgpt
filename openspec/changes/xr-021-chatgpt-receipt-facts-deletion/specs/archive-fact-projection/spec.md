# archive-fact-projection Specification

## Purpose
Turns the persisted normalized import into the `ai_archive.*` facts that Knowledge consumes, and delivers every archive event through one ack-gated publisher.

## ADDED Requirements

### Requirement: Outbox rows are complete event envelopes

Every outbox row used for the bus SHALL store the complete `EventEnvelope`, never a bare payload. The schema SHALL reject an `event_type` outside the closed list of the operation report and the eight `ai_archive.*` types.

#### Scenario: Tombstone is published as a contract envelope

- **WHEN** a normalized tombstone is enqueued and the pump runs against a real JetStream
- **THEN** the broker holds one message on `evt.ai_archive.subject.tombstoned.v1` that decodes as an `EventEnvelope` from producer `ratatoskr-chatgpt` and whose payload decodes as `AiArchiveTombstone`

#### Scenario: Unknown event type is a hard error

- **WHEN** the pump meets a row whose `event_type` is not in the closed list
- **THEN** the pass fails and the row stays unpublished

### Requirement: One pump publishes every archive event type

The publisher SHALL select unpublished rows in id order, map `event_type` through one closed mapping, publish the stored envelope unchanged with the row id as the message id, and mark `published_at` only after the acknowledgement resolves.

#### Scenario: Operation report subject is unchanged

- **WHEN** an operation report row is published
- **THEN** it goes to `evt.ai-archive.chatgpt.operation.reported.v1`

### Requirement: An import is projected from persisted rows into facts

On initial import completion and on every applied reparse, in the transaction that persists the normalized projection, the service SHALL enqueue `ai_archive.archive.imported.v1`, then one `ai_archive.project.added|updated.v1` per changed project, then one `ai_archive.conversation.added|updated.v1` per changed conversation, built from the persisted rows. Conversation content digests SHALL be computed only by the contract function, and an unchanged conversation SHALL emit nothing.

#### Scenario: Initial import of a synthetic export

- **WHEN** a synthetic export with projects and conversations is imported for a mapped account
- **THEN** the outbox holds in order one `archive.imported`, one `project.added` per project and one `conversation.added` per conversation, each passing its contract validation

#### Scenario: Reparse with identical content

- **WHEN** a reparse produces the same conversation digests
- **THEN** no outbox row is added

#### Scenario: Reparse with changed content

- **WHEN** a reparse produces a changed digest for one conversation
- **THEN** exactly one `conversation.updated` row is added

### Requirement: Fact owner is the Platform user

The owner of every `ai_archive.*` fact SHALL be `user:<Platform user>` resolved from the configured Platform account mapping. An account without a mapping SHALL emit no `ai_archive.*` fact.

#### Scenario: Unmapped account

- **WHEN** an account with no Platform mapping completes an import
- **THEN** no `ai_archive.*` fact is enqueued and the operation report still flows
