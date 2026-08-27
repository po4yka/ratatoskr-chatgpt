## MODIFIED Requirements

### Requirement: A synthetic conversations export maps to normalized records

The service SHALL recognize the documented synthetic `conversations.json` root
array for a ConsumerExport only after the existing registry selects its stable
parser identity and version. Each conversation object contains `id`, optional
`title`, optional numeric `create_time` and `update_time`, and a `mapping`
object whose values carry an optional message. A message carries `id`, optional
`parent`, `author.role`, optional numeric times, optional `metadata.model_slug`,
and `content.parts`. The parser SHALL return normalized conversation and message
records with external IDs, parent external IDs, role, timestamps, model slug,
and ordered parts. When the selected synthetic archive evidence additionally
contains documented project, Canvas, or asset records, the parser SHALL return
their typed evidence, relationships, and parser/schema stamps with the same
deterministic result; it SHALL not reconcile graph relationships or persist
rows.

#### Scenario: fixture conversations and messages map completely

- **WHEN** the committed synthetic fixture contains two conversations and its
  mapping contains messages with text, tool, and media-reference parts
- **THEN** `synthetic_fixture_maps_conversations_messages_and_parts` observes
  one normalized record per fixture conversation and message, preserving every
  part ordinal and the documented typed fields

#### Scenario: archive fixture carries project and asset evidence

- **WHEN** `synthetic_archive_fixture_maps_projects_canvas_and_asset_references`
  parses a committed synthetic archive fixture containing those documented
  records
- **THEN** the parse result preserves their supplied IDs and relationships in a
  deterministic, parser-stamped projection without treating references as bytes
