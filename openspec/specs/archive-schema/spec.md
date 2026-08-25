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
