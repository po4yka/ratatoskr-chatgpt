## Purpose

Transforms a documented synthetic conversations export into stable, loss-aware
records while retaining raw provider-shaped evidence for later parser versions.

## ADDED Requirements

### Requirement: A synthetic conversations export maps to normalized records

The service SHALL recognize the documented synthetic `conversations.json` root
array for a ConsumerExport only after the existing registry selects its stable
parser identity and version. Each conversation object contains `id`, optional
`title`, optional numeric `create_time` and `update_time`, and a `mapping`
object whose values carry an optional message. A message carries `id`, optional
`parent`, `author.role`, optional numeric times, optional `metadata.model_slug`,
and `content.parts`. The parser SHALL return normalized conversation and message
records with external IDs, parent external IDs, role, timestamps, model slug,
and ordered parts; it SHALL not reconcile graph relationships or persist rows.

#### Scenario: fixture conversations and messages map completely

- **WHEN** the committed synthetic fixture contains two conversations and its
  mapping contains messages with text, tool, and media-reference parts
- **THEN** `synthetic_fixture_maps_conversations_messages_and_parts` observes
  one normalized record per fixture conversation and message, preserving every
  part ordinal and the documented typed fields

### Requirement: Parser output identifies its interpretation

Every successful synthetic parse SHALL stamp its output with the stable schema
identifier and parser identity/version selected for that input.

#### Scenario: output has the selected parser stamp

- **WHEN** the synthetic fixture is parsed by the selected synthetic parser
- **THEN** `successful_parse_carries_schema_and_parser_version` observes the
  documented schema identifier and parser name/version on the result

### Requirement: Parser output is deterministic for identical evidence

For identical synthetic bytes and the same selected parser version, the parser
SHALL produce equal ordered normalized records and raw-preservation records on
every invocation.

#### Scenario: repeated parsing produces equal records

- **WHEN** the same committed synthetic fixture is parsed twice
- **THEN** `parsing_identical_fixture_is_deterministic` observes equal complete
  parse results without generated record identifiers or time-dependent fields

### Requirement: Unknown provider-shaped data remains recoverable

The parser SHALL not discard an unrecognized field or content part. It SHALL
retain its original JSON value with a stable source path alongside the relevant
normalized record or as an unknown ordered part. Unknown data SHALL not be
executed, rendered, dereferenced, or treated as archived asset bytes.

#### Scenario: unknown content remains raw and ordered

- **WHEN** a fixture message contains an unknown field and an unrecognized
  object in `content.parts`
- **THEN** `unknown_fields_and_parts_remain_losslessly_available` observes the
  original values at deterministic source paths and an `unknown` normalized part
  at its original ordinal
