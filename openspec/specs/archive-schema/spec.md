# archive-schema Specification

## Purpose
Defines the first version of the service's owned PostgreSQL schema and applies it to a database in one idempotent step.

## Requirements

### Requirement: The schema is one versioned definition file

The owned `chatgpt_archive` schema SHALL be defined by a single `schema.sql` at the repository root, embedded into the binary at build time, creating the `chatgpt_archive` schema and its tables for accounts, exports, import runs, projects, conversations, messages, content parts, assets, raw record preservation, completeness reports, tombstones, and outbox/inbox queues.

#### Scenario: applying creates the declared relations

- **WHEN** the schema definition is applied to an empty database
- **THEN** the `chatgpt_archive` schema exists with every table the definition declares, each carrying its primary key and ownership columns

### Requirement: Application is transactional and repeatable

Applying the schema definition to a database SHALL run inside one transaction guarded by a PostgreSQL advisory lock so concurrent starters cannot interleave, and applying it again against an already-provisioned database SHALL succeed without changing existing data.

#### Scenario: second application changes nothing

- **WHEN** the schema definition is applied twice in sequence against the same database
- **THEN** both applications succeed and the set of relations and their definitions are unchanged after the second run

### Requirement: No migration tooling exists

Schema evolution SHALL edit `schema.sql` in place; the service SHALL NOT create or carry any migration files, migration runner, or version negotiation.

#### Scenario: no migration artifacts ship

- **WHEN** the repository tree is inspected
- **THEN** no migrations directory, migration manifest, or migration runner configuration exists anywhere under version control

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

### Requirement: Current schema records privacy deletion and reparse lifecycles

The single current `schema.sql` SHALL define explicit export-to-entity observations, persisted extracted-artifact references, tenant-owned privacy deletion requests, deletion inventory items, content-free deletion audit outcomes, reparse runs, and parser migration reports with idempotency constraints, terminal-state checks, correlation identifiers, and foreign keys only within `chatgpt_archive`. Schema changes SHALL be made in place and SHALL remain repeatably applicable.

#### Scenario: Fresh schema exposes lifecycle relations and constraints

- **WHEN** the current schema is applied twice to a fresh PostgreSQL database
- **THEN** all privacy deletion, reparse, and parser migration relations exist once with their tenant ownership, uniqueness, state, and local foreign-key constraints intact

### Requirement: Retained audit does not retain deleted private content

Deletion audit and report records SHALL retain only internal request or subject identifiers, scope class, parser identity where applicable, category counts, timestamps, correlation identifiers, terminal outcome, non-sensitive error codes, and the `BlobRef` of a content-free deletion evidence document. They SHALL NOT retain message bodies, titles, filenames, raw payloads, external account references, source archive digests, or source-content blob references.

#### Scenario: Tenant deletion leaves only content-free audit evidence

- **WHEN** tenant deletion completes and the owned schema is queried for the deleted tenant
- **THEN** no source or normalized content remains and the surviving audit row contains only the allowed operational fields
