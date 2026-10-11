## ADDED Requirements

### Requirement: A duplicate of a permanently failed import is reported failed

When an upload duplicates an export whose import run is `failed`, the new operation SHALL receive the same `failed` report (`chatgpt.archive.invalid`, not retryable) exactly once, and a later pass SHALL queue no further report.

#### Scenario: Second upload of corrupt bytes

- **WHEN** the same admitted-but-corrupt archive is uploaded under a second operation after its first import failed
- **THEN** after one pass the second operation has a queued `failed` report, and a further pass queues none

### Requirement: A permanently failed import is logged

A run marked `failed` SHALL be logged at WARN with the closed class `archive_invalid`, `parse_failed` or `reparse_conflict` and the run id, and never an archive path, name or content.

#### Scenario: Failed import leaves a line

- **WHEN** an import run is marked failed
- **THEN** one WARN event with the class and the run id is emitted
