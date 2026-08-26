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
