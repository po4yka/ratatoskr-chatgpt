## ADDED Requirements

### Requirement: A deletion asks Edge to erase the facts it removed from the outbox

Every path that removes a published, content-bearing fact row (`ai_archive.archive.imported`, `conversation.added|updated`, `project.added|updated`, `artifact.added|updated`) SHALL collect its `published_stream_seq` before the row is gone and SHALL, in the same transaction, enqueue one `platform.stream_erasure.requested.v1` command per distinct owner with at most 500 ascending sequences each. A tombstone, an operation report and an unpublished fact SHALL NOT contribute a sequence. The command SHALL be a complete `CommandEnvelope` whose producer is `ratatoskr-chatgpt`, whose `aggregate_id` is `privacy_request:<request id>` and whose `tenant_id` equals the payload `owner`.

#### Scenario: Archive deletion

- **WHEN** an archive whose conversation and project facts were published is deleted
- **THEN** one command per owner lists the sequences of all those facts, including superseded `conversation.updated` rows, in ascending order

#### Scenario: More than 500 sequences

- **WHEN** a deletion removes more than 500 published facts of one owner
- **THEN** the sequences are split into several commands of at most 500, each ascending

#### Scenario: Unpublished facts

- **WHEN** a deleted fact was never published
- **THEN** it adds no sequence and no command

#### Scenario: A later deletion

- **WHEN** a tenant-scope deletion runs while an erasure command is unpublished
- **THEN** the command row remains
