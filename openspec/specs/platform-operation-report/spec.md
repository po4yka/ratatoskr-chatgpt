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
