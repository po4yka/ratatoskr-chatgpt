## ADDED Requirements

### Requirement: The pump records the stream sequence of every published row

When the pump marks a row published it SHALL store the stream sequence of the broker acknowledgement in `published_stream_seq`; a duplicate acknowledgement SHALL store the original sequence. The pump SHALL map `platform.stream_erasure.requested.v1` to the subject `cmd.platform.stream_erasure.requested.v1` and publish its stored `CommandEnvelope` with the `command_id` as `Nats-Msg-Id`.

#### Scenario: A published fact

- **WHEN** the pump publishes a fact
- **THEN** `published_stream_seq` equals the sequence of the stored stream message

#### Scenario: An erasure command

- **WHEN** an erasure command row is pending
- **THEN** the pump publishes it to `cmd.platform.stream_erasure.requested.v1`
