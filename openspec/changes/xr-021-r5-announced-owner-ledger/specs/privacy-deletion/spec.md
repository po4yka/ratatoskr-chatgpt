## MODIFIED Requirements

### Requirement: Tombstones are complete envelopes owned by the Platform user

A user-requested deletion SHALL enqueue, for each deleted archive, conversation and project, one `ai_archive.subject.tombstoned.v1` envelope per distinct owner the announced-subject ledger holds for that subject, addressed to that owner, with reason `user_requested`. The owner SHALL be read from `chatgpt_archive.announced_subjects` inside the row-removal transaction and SHALL NOT be read from the live `PLATFORM_ACCOUNTS` mapping or from outbox envelopes. A subject with no ledger row SHALL emit no tombstone, SHALL NOT report a gap and SHALL NOT fail the request. The ledger rows of a tombstoned subject SHALL be deleted in the same transaction and no other subject's rows SHALL be touched. A tenant deletion SHALL keep unpublished tombstones of earlier requests. Survivors of the deletion SHALL be announced again under the owner of the retained archive's ledger row.

#### Scenario: Archive deletion tombstone

- **WHEN** an archive of a mapped account is deleted
- **THEN** the outbox row decodes as an `EventEnvelope` whose tenant and payload owner are `user:<Platform user>`, and whose evidence blob exists

#### Scenario: The mapping changed after the import

- **WHEN** an archive was announced under `user:A` and the mapping no longer contains its account, or names another user, and the archive is deleted
- **THEN** the tombstones name `user:A` and the completion totals hold no `downstream_tombstone_unbound`

#### Scenario: Two owners for one conversation

- **WHEN** two archives of one account were announced under `user:A` and `user:B`, both announced one conversation, and a tenant-scope deletion runs
- **THEN** one tombstone names the conversation for `user:A`, one for `user:B`, each archive tombstone names its own owner, and no tombstone names a third user

#### Scenario: Nothing was announced

- **WHEN** an account that never announced a fact has an archive deleted
- **THEN** the deletion completes with no tombstone and no `downstream_tombstone_unbound`

#### Scenario: The ledger is cleaned with the subject

- **WHEN** a subject is tombstoned
- **THEN** its ledger rows are gone and the ledger rows of every other subject remain
