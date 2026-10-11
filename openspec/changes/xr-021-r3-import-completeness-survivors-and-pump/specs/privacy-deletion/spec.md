## ADDED Requirements

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
