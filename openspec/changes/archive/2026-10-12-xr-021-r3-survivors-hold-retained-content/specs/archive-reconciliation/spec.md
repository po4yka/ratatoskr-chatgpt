## ADDED Requirements

### Requirement: Each export records the version of a shared record it presented

An export that observes a project or conversation SHALL record the digest of the version it presented, and the service SHALL keep one snapshot per distinct version for as long as some export presents it. A later export that reads a conversation with unchanged content SHALL observe the messages it presented and no message that only an earlier version held.

#### Scenario: Unchanged read does not adopt earlier messages

- **WHEN** a later export holds a conversation without a message that an earlier export held
- **THEN** the later export observes the conversation and its own messages, and not the earlier-only message
