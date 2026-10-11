## ADDED Requirements

### Requirement: A surviving record holds only content a retained archive presented

After a privacy deletion, every project and conversation that remains SHALL hold only content that a retained export presented, in its row, its content-part and revision history, its stored version snapshots and the fact announced for it. When a record keeps a version that no retained export presented, the row-removal transaction SHALL rebuild it from the versions retained exports presented before announcing it again: a project from the newest retained version, a conversation from the retained versions in export order. The announcing fact SHALL name the earliest retained export that presented the content the record holds.

#### Scenario: Conversation edited by the deleted export

- **WHEN** export E1 holds a conversation and export E2 holds it with edited text and E2 is deleted
- **THEN** neither the database nor any fact queued after the tombstone contains the edited text, and the conversation is announced again with E1's text under E1

#### Scenario: Project edited by the deleted export

- **WHEN** export E2 changed the title and description of a project that E1 also holds and E2 is deleted
- **THEN** the project row, its snapshots and every queued fact carry E1's title and description

#### Scenario: Intermediate version

- **WHEN** exports E1, E2 and E3 hold three different texts of one conversation and E2 is deleted
- **THEN** E2's text is gone, the content-part history holds E1's and E3's text in order, and E3's text stays current

#### Scenario: A retained export repeats the edit

- **WHEN** E1 holds the original, E2 the edit and E3 the same edit, and E2 is deleted
- **THEN** the edit stays current and the survivor fact names E3, not E1

#### Scenario: Tenant deletion

- **WHEN** every export of the tenant is deleted
- **THEN** no version snapshot of the tenant remains
