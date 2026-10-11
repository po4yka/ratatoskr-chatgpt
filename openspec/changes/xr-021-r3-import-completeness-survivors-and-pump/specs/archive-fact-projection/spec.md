## ADDED Requirements

### Requirement: The pump keeps one broker connection and names messages by event id

The pump SHALL hold one lazily created connection for its lifetime, SHALL perform no network call for a pass with no pending row, SHALL replace a connection the client reports closed, and SHALL publish each message with `Nats-Msg-Id` equal to the envelope `event_id`.

#### Scenario: Many passes, one connection

- **WHEN** twenty passes run, some with pending rows and some without
- **THEN** the broker's total connection count grows by at most one

#### Scenario: Empty pass with the broker down

- **WHEN** the broker is stopped and a pass finds no pending row
- **THEN** the pass returns zero without an error

### Requirement: The pump uses its own inbox prefix

Every NATS connection of the service SHALL set the inbox prefix `_INBOX_chatgpt`, so the pump completes JetStream requests against a broker that lets the identity subscribe only to `_INBOX_chatgpt.>`.

#### Scenario: Restricted broker

- **WHEN** the pump runs against a broker whose only user is the stanza of `deploy/nats/identity.conf`
- **THEN** the pending row is published with an acknowledgement and marked published
