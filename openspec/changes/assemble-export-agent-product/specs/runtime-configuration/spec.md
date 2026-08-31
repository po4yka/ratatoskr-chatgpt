## ADDED Requirements

### Requirement: Operation reports require a configured NATS NKey identity

Archive operation reporting SHALL require a validated NATS NKey seed file in addition to the bus
URL. The publisher SHALL authenticate with that file and SHALL have no anonymous fallback.

#### Scenario: The NKey seed is absent

- **WHEN** archive receipt and reporting are configured without a readable NKey seed file
- **THEN** startup fails closed without serving the archive receipt route

### Requirement: Readiness follows import-worker and report-publisher health

When archive receipt is configured, readiness SHALL include the supervised import worker and an
authenticated report publisher that can reach its permitted subject. Publication failure SHALL keep
durable rows pending and make readiness fail until a successful authenticated pass recovers it.

#### Scenario: Broker permission is revoked

- **WHEN** the broker refuses the ChatGPT publisher's operation-report subject
- **THEN** the row remains pending and `/health/ready` reports the publisher dependency not ready
