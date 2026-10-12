## ADDED Requirements

### Requirement: The announced-subject ledger records the owner each fact named

`schema.sql` SHALL define `chatgpt_archive.announced_subjects` with `subject_kind` (`archive`, `conversation` or `project`), `subject_id`, `ai_archive_id`, `owner_ref` (a `user:<uuid>` text), `account_id` and `announced_at`, primary key `(subject_kind, subject_id, owner_ref)`. It SHALL have no foreign key to `exports`. A row SHALL be inserted in the transaction that enqueues the fact that announces the subject.

#### Scenario: Import writes the ledger

- **WHEN** an import of a mapped account completes
- **THEN** the ledger holds one row for the archive, one per conversation and one per project, each with the mapped owner

#### Scenario: An unmapped account writes nothing

- **WHEN** an import of an account without a mapping completes
- **THEN** no fact is enqueued and the ledger holds no row

#### Scenario: Deleting an export keeps the ledger

- **WHEN** an export row is deleted
- **THEN** the ledger rows announced under it remain
