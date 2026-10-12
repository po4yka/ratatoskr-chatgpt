## ADDED Requirements

### Requirement: A deletion keeps every unacknowledged tombstone

No deletion SHALL remove a `ai_archive.subject.tombstoned.v1` outbox row whose `acknowledged_at` is null, whether or not it was published. An acknowledged tombstone MAY be removed with the tenant's rows.

#### Scenario: A later tenant deletion

- **WHEN** a tenant-scope deletion runs while an earlier tombstone is published and not acknowledged
- **THEN** the earlier tombstone row remains

#### Scenario: An acknowledged tombstone

- **WHEN** a tenant-scope deletion runs and an earlier tombstone is acknowledged
- **THEN** the earlier tombstone row is removed with the tenant's other outbox rows
