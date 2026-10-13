## MODIFIED Requirements

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
