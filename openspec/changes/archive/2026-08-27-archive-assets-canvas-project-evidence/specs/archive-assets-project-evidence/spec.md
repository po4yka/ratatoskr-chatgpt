## Purpose

Preserves project-scoped evidence and referenced archive assets without treating
unavailable bytes, unsafe content, or omitted snapshots as successful backup.

## ADDED Requirements

### Requirement: Projects and instructions remain first-class evidence

When a supported export supplies a project record, the service SHALL preserve
its stable external ID, title/description, observed lifecycle and membership
metadata, conversation references, and every supplied instruction or system
prompt as ordered evidence. It SHALL retain unknown project-shaped fields at a
stable source path, SHALL NOT invent a project or instruction from a missing
field, and SHALL stamp the record with the selected schema and parser version.

#### Scenario: project instructions survive fixture parsing

- **WHEN** `project_and_instruction_evidence_is_preserved` parses a fixture
  with a project, an instruction, a system prompt, and one linked conversation
- **THEN** the parsed result exposes each supplied value and relationship under
  the project external ID, with its parser/schema stamps and without creating
  records for absent project fields

### Requirement: Canvas documents preserve supplied content safely

When a supported export supplies a Canvas-like document, the service SHALL
preserve its stable external ID, project or conversation relationship when
present, ordered document content, observed metadata, and raw unknown fields.
It SHALL treat the document as inert evidence: it SHALL NOT render active HTML,
execute code, fetch external references, or claim a local byte archive when the
export carries only a reference.

#### Scenario: Canvas evidence stays inert and linked

- **WHEN** `canvas_document_content_is_preserved_as_evidence` parses a fixture
  containing a Canvas document linked to a project and conversation
- **THEN** the result retains the supplied content and relationships while
  exposing no preview, execution result, or fabricated local asset reference

### Requirement: Referenced archive assets are verified before association

For every file or generated-asset reference supplied by a supported export, the
service SHALL record its stable provider ID, display metadata, ownership
relationships, and whether archive bytes were evidenced. If an archive entry is
matched, it SHALL verify the candidate `BlobRef` against its owner, SHA-256
digest, length, and media type before associating it. An unverified, mismatched,
or extraction-quarantined candidate SHALL remain quarantined and SHALL NOT be
reported as archived; a reference with no supplied bytes SHALL remain missing.

#### Scenario: digest mismatch quarantines an asset

- **WHEN** `asset_digest_mismatch_is_quarantined` processes a fixture asset
  whose declared digest differs from the matched extracted bytes
- **THEN** the asset is retained with a quarantine status and anomaly code, has
  no usable archived `BlobRef`, and does not cause the parser to fetch or render
  the referenced content

#### Scenario: verified bytes retain their BlobRef

- **WHEN** `verified_asset_keeps_its_blob_reference` processes a fixture asset
  whose archive entry matches the declared digest, length, and media type
- **THEN** the asset retains the verified `BlobRef`, distinguishes uploaded from
  generated provenance, and records the supplied owner relationships

### Requirement: Project and asset evidence reconciles without destructive absence inference

For ordered archive snapshots, the service SHALL append or reuse deterministic
revisions for projects, instructions, Canvas documents, and assets by stable
provider ID and complete normalized evidence. It SHALL record an explicit
missing-from-latest observation when a previously evidenced record is absent
from a later snapshot, retain earlier evidence, and SHALL NOT infer deletion,
lost access, or asset byte availability from that omission.

#### Scenario: later project omission preserves prior evidence

- **WHEN** `missing_project_evidence_is_an_observation_not_a_deletion`
  reconciles two snapshots where the second omits a previously evidenced project
- **THEN** the earlier project/instruction revisions remain available and the
  later snapshot carries a non-destructive missing observation

### Requirement: Completeness reports expose asset and project evidence gaps

Each archive-local and cumulative completeness report SHALL count observed
projects, instructions, Canvas documents, asset references, verified archived
assets, missing assets, and quarantined assets. It SHALL classify project,
Canvas, and asset coverage conservatively and SHALL NOT classify an archive as
complete when any supplied reference is missing or quarantined, or when the
parser supplied no evidence for that category. Reports SHALL exclude titles,
instructions, filenames, document content, digests, and raw provider values.

#### Scenario: quarantined asset keeps report partial

- **WHEN** `quarantined_asset_keeps_completeness_partial` reconciles a fixture
  snapshot with one project and one quarantined asset
- **THEN** the report exposes the correct structured counts and a partial
  coverage class without exposing any private project or asset content
