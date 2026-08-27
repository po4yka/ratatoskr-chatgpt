# parser-registry Specification

## Purpose

Selects only explicitly registered, versioned archive parsers from structural
inspection evidence while preserving an unambiguous unsupported outcome.

## Requirements

### Requirement: Parsers declare stable identity and capabilities

Every parser registered with the service SHALL declare a stable parser
identifier, a version, and the archive structure and acquisition capabilities
it accepts. A registry SHALL refuse duplicate parser identities rather than
silently replacing an existing parser.

#### Scenario: duplicate parser identity is refused

- **WHEN** a second parser is registered with the same identifier and version
  as an existing parser
- **THEN** registration returns a typed duplicate-identity outcome and the
  original registration remains selectable

### Requirement: Structural selection is deterministic and conservative

Given an inspected archive and acquisition mode, the registry SHALL select a
parser only when exactly one registered declaration supports that structure.
No match and more than one match SHALL produce distinct explicit outcomes and
SHALL NOT invoke a parser.

#### Scenario: matching structure selects its one parser

- **WHEN** one registered parser declares support for the inspected ZIP's
  required structural signals and acquisition mode
- **THEN** selection returns that parser's stable identifier and version

#### Scenario: unsupported structure is preserved as unsupported

- **WHEN** no registered parser declares support for an inspected archive
- **THEN** selection returns an explicit unsupported outcome with the
  inspection evidence available for a later parser version

#### Scenario: overlapping declarations are ambiguous

- **WHEN** two registered parsers declare support for the same inspected
  archive and acquisition mode
- **THEN** selection returns an explicit ambiguous outcome naming both parser
  identities and invokes neither parser

### Requirement: Registry resolves exact parser identities for operator workflows

The registry SHALL support exact lookup by parser name and declared parser version and SHALL expose compatible registered identities in deterministic version order for a supplied acquisition mode and inspected archive. Exact lookup SHALL refuse an identity whose declared signals do not match the inspected evidence.

#### Scenario: Exact compatible parser resolves once

- **WHEN** reparse requests a registered parser identity compatible with the archive's acquisition mode and signals
- **THEN** the registry returns exactly that parser identity and its executable parser without applying automatic selection

#### Scenario: Compatible versions have deterministic order

- **WHEN** compatible parser registrations were inserted in different orders
- **THEN** version discovery returns the same unique identities in declared version order

### Requirement: Automatic intake remains ambiguity-safe

Adding exact lookup and version discovery SHALL NOT cause ordinary intake to silently pick a parser when multiple declarations match. Automatic selection SHALL continue to return every ambiguous identity and perform no parse.

#### Scenario: Two compatible versions remain ambiguous at intake

- **WHEN** ordinary intake sees two matching versions and no exact operator target
- **THEN** selection returns an ambiguity containing both identities and neither parser executes
