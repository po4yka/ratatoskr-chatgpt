## ADDED Requirements

### Requirement: Service startup resumes the persisted import pipeline

The service SHALL supervise restart-safe workers that advance stored work through the existing
inspection, parser, reconciliation, persistence, and completeness stages. Every accepted stage SHALL
be durable before the next stage starts, and a restart SHALL resume from the last durable stage.

#### Scenario: Stored work survives a restart

- **WHEN** the process restarts with a Platform-bound run whose raw bytes are stored but not parsed
- **THEN** a worker resumes it without a re-upload and persists the actual normalized result

### Requirement: Terminal completeness is derived from imported evidence

Terminal state SHALL be selected only from the actual parser and reconciliation outcome. Missing or
malformed evidence SHALL remain failed or explicitly incomplete and SHALL NOT be converted to a
successful or gap-free result.

#### Scenario: Synthetic import produces bounded truth

- **WHEN** the supported synthetic ChatGPT export completes parsing and reconciliation
- **THEN** the stored terminal result contains the observed counts and completeness classification
