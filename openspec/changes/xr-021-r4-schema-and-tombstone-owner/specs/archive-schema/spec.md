## ADDED Requirements

### Requirement: The definition carries no column additions after table creation

`schema.sql` SHALL declare every column inside its `CREATE TABLE` statement and SHALL NOT add a column with `ALTER TABLE ... ADD COLUMN`, so applying the definition emits no `already exists` notice from an `ALTER TABLE`.

#### Scenario: A scan finds no column addition

- **WHEN** `schema.sql` is scanned for `ALTER TABLE ... ADD COLUMN`
- **THEN** no statement is found

#### Scenario: Repeated application is silent

- **WHEN** the definition is applied twice to one database
- **THEN** the server emits no notice that a column already exists
