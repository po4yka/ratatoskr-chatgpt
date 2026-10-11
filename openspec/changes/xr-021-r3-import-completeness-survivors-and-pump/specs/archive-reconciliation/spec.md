## ADDED Requirements

### Requirement: Cumulative exports share one row per conversation

A conversation SHALL be one record per account and provider conversation id however many exports contain it. Each export that holds it SHALL observe that record and its messages, an export that reads it with unchanged content SHALL add no revision and announce no fact, and a changed conversation SHALL keep its identity and gain a revision.

#### Scenario: Two exports, one conversation

- **WHEN** a second export of the same account holds a conversation the first export held
- **THEN** the account has one conversation row for it, both exports observe it, and the second export announces no `added` fact for it

#### Scenario: An edited conversation

- **WHEN** a later export holds the same conversation with different content
- **THEN** the row keeps its id, a second revision is recorded and the fact announced for it is an `updated` fact
