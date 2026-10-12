## MODIFIED Requirements

### Requirement: Tombstones are complete envelopes owned by the Platform user

A user-requested deletion SHALL enqueue one `ai_archive.subject.tombstoned.v1` envelope per deleted archive, conversation and project, owned by the Platform user the archive's own announced facts named, with reason `user_requested`. The owner SHALL be read from the announced facts in the outbox inside the row-removal transaction, before the matching rows are removed: the latest `ai_archive.archive.imported.v1` fact of a deleted archive, else the first announced fact of a deleted conversation or project, else the latest imported fact of the account, else any announced fact of the account. The live `PLATFORM_ACCOUNTS` mapping SHALL be consulted only when no announced fact gives an owner. An account that announced a fact but yields no owner SHALL emit none and the completion totals SHALL report `downstream_tombstone_unbound`. An account that announced nothing and has no mapping SHALL emit none and SHALL NOT report a gap. A tenant deletion SHALL keep unpublished tombstones of earlier requests. Survivors of the deletion SHALL be announced again under the same owner.

#### Scenario: Archive deletion tombstone

- **WHEN** an archive of a mapped account is deleted
- **THEN** the outbox row decodes as an `EventEnvelope` whose tenant and payload owner are `user:<Platform user>`, and whose evidence blob exists

#### Scenario: The mapping changed after the import

- **WHEN** an archive was announced under `user:A` and the mapping no longer contains its account, or names another user, and the archive is deleted
- **THEN** the tombstones name `user:A`, no item fails, and the completion totals hold no `downstream_tombstone_unbound`

#### Scenario: Nothing was announced

- **WHEN** an account with no announced fact and no mapping has an archive deleted
- **THEN** the deletion completes with no tombstone and no `downstream_tombstone_unbound`

#### Scenario: Announced fact without an owner

- **WHEN** the only announced fact of an account carries no owner and the account has no mapping
- **THEN** the deletion completes with no tombstone and the completion totals report `downstream_tombstone_unbound`
