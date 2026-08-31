## ADDED Requirements

### Requirement: Every bound operation receives one import-terminal report

Each Platform operation associated with an archive SHALL receive one idempotent terminal
`platform.operation.reported.v1` document only after the durable import result exists. Reusing an
existing archive/import SHALL still create the operation-specific outbox identity.

#### Scenario: Duplicate archive reports a reused result

- **WHEN** a second Platform operation is bound to bytes whose import already completed
- **THEN** the service enqueues one terminal report for that operation citing the existing result

### Requirement: ChatGPT reports use the ChatGPT ingress subject

The publisher SHALL send the unchanged operation-report document only on
`evt.ai-archive.chatgpt.operation.reported.v1` with the stable ChatGPT producer identity.

#### Scenario: A pending report is published

- **WHEN** the configured authenticated publisher receives broker acknowledgement
- **THEN** the outbox row is marked published only after acknowledgement on the ChatGPT subject
