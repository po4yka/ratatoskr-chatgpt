## ADDED Requirements

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
