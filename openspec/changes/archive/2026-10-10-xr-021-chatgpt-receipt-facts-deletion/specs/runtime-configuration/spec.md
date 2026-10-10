## ADDED Requirements

### Requirement: The operator listener defaults to 9085

The operator listener SHALL default to `127.0.0.1:9085` so it does not collide with the Threads and Claude operator listeners.

#### Scenario: Default listener

- **WHEN** configuration loads with no listener override
- **THEN** the operator listen address is `127.0.0.1:9085`
