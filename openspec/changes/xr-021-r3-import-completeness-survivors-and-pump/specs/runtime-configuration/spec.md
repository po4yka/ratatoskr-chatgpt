## ADDED Requirements

### Requirement: The operator example names a provisioned role

The role and database named by `deploy/systemd/chatgpt.conf.example` SHALL be the ones `deploy/postgres/role.sql` creates: role `ratatoskr_chatgpt` on database `ratatoskr`, port 5432.

#### Scenario: Example and role file agree

- **WHEN** the example's database URL is compared with `deploy/postgres/role.sql`
- **THEN** the role, the database and the port match
