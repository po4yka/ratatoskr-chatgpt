# archive-schema delta

## ADDED Requirements

### Requirement: Exports are tenant-scoped immutable receipt records

The `chatgpt_archive.exports` relation SHALL record, for every received export, a non-null owning account, an acquisition mode from the supported set, the fleet blob reference JSON of the immutable original, the SHA-256 hex digest unique together with the owning account, the byte length, and distinct receive/import-start timestamps.

#### Scenario: equal digests may coexist across accounts but not within one

- **WHEN** exports are inserted with identical digests for two different accounts
- **THEN** both inserts succeed; inserting a second export with the same digest for the same account is rejected by the schema constraint

#### Scenario: an export always names its owner

- **WHEN** an export row is inserted without an account
- **THEN** the insert is rejected by the not-null constraint on the owning account

### Requirement: Import runs carry resumable machine state

The `chatgpt_archive.import_runs` relation SHALL admit exactly the states declared by the import-state capability, SHALL allow a run row to exist before its export row materializes (nullable export link), SHALL carry the run's digest and byte length once hashing completes, and SHALL NOT require a parser version before parsing exists.

#### Scenario: a run can exist before its export row

- **WHEN** an import-run row is inserted referencing no export, at state `received`
- **THEN** the insert succeeds; recording the state value `unknown_stage` is rejected by the state constraint

#### Scenario: runs capture digest and length at the hashed stage

- **WHEN** a run row is updated into `hashed` with its digest and byte length
- **THEN** the values persist and remain queryable after restart, providing the evidence resume needs
