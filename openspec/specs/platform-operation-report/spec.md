## Purpose

Publishes one privacy-safe terminal operation fact for every Platform-forwarded
ChatGPT archive receipt without overstating parser completeness.

## Requirements

### Requirement: Edge-minted archive receipt reports a terminal import fact

The ChatGPT archive service SHALL accept Platform archive bytes only through its loopback receipt
endpoint with complete Edge-minted claims. It SHALL resolve the minted user through an explicit
account mapping, preserve and verify raw bytes before reporting, and publish exactly one terminal
`platform.operation.reported.v1` event for the supplied operation identifier.

#### Scenario: Raw archive stored but completeness is not yet established

- **WHEN** the claimed digest and size match bytes that the receipt stores durably and no parser
  completeness fact exists
- **THEN** the report is `partially_succeeded` with an `ai_archive.import` result summary whose
  completeness is `unknown`, rather than claiming a complete import

#### Scenario: Missing minted claim stores nothing

- **WHEN** a direct or incomplete request reaches the receipt endpoint
- **THEN** it is refused before account lookup, raw storage, or operation reporting

### Requirement: An incomplete import reports the contract warning

An import whose completeness is not `complete` SHALL be reported as `partially_succeeded` with exactly one warning, the contract incomplete-import warning, and no error. A complete import SHALL be reported as `succeeded` without warnings. Every report the service emits SHALL satisfy the contract report invariant.

#### Scenario: Structurally partial import

- **WHEN** an import with completeness `structurally_partial` is reported
- **THEN** the report is `partially_succeeded`, carries the `ai_archive.import.incomplete` warning and passes `OperationReported::validate`

#### Scenario: Complete import

- **WHEN** an import with completeness `complete` is reported
- **THEN** the report is `succeeded` and carries no warning
