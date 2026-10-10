## ADDED Requirements

### Requirement: An incomplete import reports the contract warning

An import whose completeness is not `complete` SHALL be reported as `partially_succeeded` with exactly one warning, the contract incomplete-import warning, and no error. A complete import SHALL be reported as `succeeded` without warnings. Every report the service emits SHALL satisfy the contract report invariant.

#### Scenario: Structurally partial import

- **WHEN** an import with completeness `structurally_partial` is reported
- **THEN** the report is `partially_succeeded`, carries the `ai_archive.import.incomplete` warning and passes `OperationReported::validate`

#### Scenario: Complete import

- **WHEN** an import with completeness `complete` is reported
- **THEN** the report is `succeeded` and carries no warning
