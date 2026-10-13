# archive-fact-projection Specification

## Purpose
Turns the persisted normalized import into the `ai_archive.*` facts that Knowledge consumes, and delivers every archive event through one ack-gated publisher.

## Requirements

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

### Requirement: A fact over the event limit is not enqueued and the import says so

A fact whose serialized envelope is larger than `RATATOSKR__RECEIPT__MAX_EVENT_BYTES` SHALL NOT be enqueued. The terminal operation report written in the same transaction SHALL be `partially_succeeded` with the warning `ai_archive.facts.too_large`, in addition to the one `ai_archive.import.incomplete` warning when the import is also incomplete.

#### Scenario: One oversized conversation

- **WHEN** an import contains a conversation whose `ai_archive.conversation.added.v1` envelope exceeds the configured limit and another that does not
- **THEN** only the smaller conversation fact is enqueued and the report is `partially_succeeded` with the too-large warning and passes `validate()`

### Requirement: An undeliverable outbox row never wedges the pump

A row whose body is larger than the connected broker `max_payload` minus 1 KiB SHALL be marked undeliverable with `last_error = 'payload_too_large'`, SHALL be skipped from then on, and SHALL NOT make the pass fail or the service not ready. A transient refusal SHALL never mark a row undeliverable.

#### Scenario: Oversized row between two small rows

- **WHEN** the pump runs against a broker with a small `max_payload` and the middle row is larger than the limit
- **THEN** the first and last rows are published, the middle row is undeliverable, the pass returns success and readiness stays true

### Requirement: The pump keeps one broker connection and names messages by event id

The pump SHALL hold one lazily created connection for its lifetime, SHALL perform no network call for a pass with no pending row, SHALL replace a connection the client reports closed, and SHALL publish each message with `Nats-Msg-Id` equal to the envelope `event_id`.

#### Scenario: Many passes, one connection

- **WHEN** twenty passes run, some with pending rows and some without
- **THEN** the broker's total connection count grows by at most one

#### Scenario: Empty pass with the broker down

- **WHEN** the broker is stopped and a pass finds no pending row
- **THEN** the pass returns zero without an error

### Requirement: The pump uses its own inbox prefix

Every NATS connection of the service SHALL set the inbox prefix `_INBOX_chatgpt`, so the pump completes JetStream requests against a broker that lets the identity subscribe only to `_INBOX_chatgpt.>`.

#### Scenario: Restricted broker

- **WHEN** the pump runs against a broker whose only user is the stanza of `deploy/nats/identity.conf`
- **THEN** the pending row is published with an acknowledgement and marked published

### Requirement: A tombstone is re-sent until Knowledge acknowledges it

When the pump marks a tombstone published it SHALL set `next_resend_at` to now plus the configured resend interval. A published tombstone with no `acknowledged_at` whose `next_resend_at` has passed SHALL be published again with the stored bytes unchanged and the same `Nats-Msg-Id`, its `resend_count` SHALL increase by one and its `next_resend_at` SHALL become now plus the lesser of the interval times two to the old count and the configured cap. A failed republish SHALL leave the row and use the failure backoff. The resend SHALL stop only when `acknowledged_at` is set.

#### Scenario: No acknowledgement arrives

- **WHEN** a tombstone was published and the resend interval passes without an acknowledgement
- **THEN** the same bytes are published again with the same `Nats-Msg-Id` and the next resend waits at most twice as long, never longer than the cap

#### Scenario: The acknowledgement arrives

- **WHEN** Knowledge publishes an `applied` acknowledgement naming the tombstone's event id and owner
- **THEN** `acknowledged_at` is set and the row is never published again

#### Scenario: A refusal

- **WHEN** Knowledge publishes a `refused` acknowledgement
- **THEN** the resend stops, an ERROR line is logged and `chatgpt_archive_tombstone_refused_total` increases

#### Scenario: An acknowledgement that names no row

- **WHEN** an acknowledgement names an unknown event id or another owner's row
- **THEN** no row changes and the message is acknowledged to the broker

#### Scenario: Knowledge is down

- **WHEN** no acknowledgement ever arrives
- **THEN** the service stays ready and the unacknowledged-tombstone gauges grow

### Requirement: The pump records the stream sequence of every published row

When the pump marks a row published it SHALL store, in `published_stream_seqs bigint[]`, the stream sequence of every broker acknowledgement it received for that row, ascending and without duplicates; a duplicate acknowledgement SHALL add nothing. The first publication of a row SHALL be one transaction that locks the row with `FOR UPDATE SKIP LOCKED`, publishes, marks and commits; a row that is gone or locked SHALL be skipped and nothing published for it. The mark SHALL affect exactly one row; any other count SHALL be counted in `chatgpt_archive_outbox_mark_lost_total` and returned as an error. The pump SHALL set the JetStream request timeout to 5 s. A broker acknowledgement whose commit failed SHALL be carried and merged by the next successful mark of that row; a carried entry whose row no longer exists SHALL be turned into a stream erasure command by the pump at the start of the next pass. The pump SHALL map `platform.stream_erasure.requested.v1` to the subject `cmd.platform.stream_erasure.requested.v1` and publish its stored `CommandEnvelope` with the `command_id` as `Nats-Msg-Id`.

#### Scenario: A published fact

- **WHEN** the pump publishes a fact
- **THEN** `published_stream_seqs` equals the one sequence of the stored stream message

#### Scenario: A duplicate acknowledgement

- **WHEN** the same row is published again inside the stream's duplicate window
- **THEN** `published_stream_seqs` still holds one element

#### Scenario: A mark that lands nowhere

- **WHEN** the mark finds no unpublished row with that id
- **THEN** the pump returns `MarkLost` and increments `chatgpt_archive_outbox_mark_lost_total`

#### Scenario: Deleted before the lock

- **WHEN** a deletion commits after the pump chose its candidates and before it locks the row
- **THEN** nothing of the deleted fact is published

#### Scenario: Republished after a failed commit

- **WHEN** a commit fails after the acknowledgement and the row is republished after the duplicate window
- **THEN** `published_stream_seqs` holds both sequences, ascending

#### Scenario: A carried copy whose row was deleted

- **WHEN** the row of a carried acknowledgement is gone at the next pass
- **THEN** the pump enqueues one erasure command for the carried sequences and enqueues nothing more on a later pass

#### Scenario: An erasure command

- **WHEN** an erasure command row is pending
- **THEN** the pump publishes it to `cmd.platform.stream_erasure.requested.v1`
