## Purpose

Safely inventories and extracts received archive evidence without executing
content, accepting unsafe paths or limits, or losing raw-archive provenance.

## ADDED Requirements

### Requirement: Inspection produces a bounded structural inventory

The service SHALL inspect a supported archive from verified raw BlobRef
evidence without executing, rendering, or interpreting its entry content. The
inventory SHALL report normalized entry names, entry kind, compressed and
declared uncompressed byte counts, aggregate counts, and structural type
signals sufficient for parser selection.

#### Scenario: ZIP inspection reports JSON, HTML, and media structure

- **WHEN** a verified ZIP contains `conversations.json`, `chat.html`, and an
  image entry within every configured limit
- **THEN** inspection reports all three normalized names and their detected
  structural types without rendering or executing any entry

### Requirement: Unsafe or ambiguous archives are rejected before extraction

The service SHALL reject an archive before it extracts bytes when an entry is
absolute, traverses a parent path, has an empty or duplicate normalized name,
represents a special file, or exceeds the configured count, per-entry,
aggregate decompressed-size, compressed-size, or compression-ratio limits.

#### Scenario: traversal entry is rejected

- **WHEN** a ZIP contains an entry named `../outside.json`
- **THEN** inspection returns a typed unsafe-path outcome and extraction
  creates no artifact BlobRef

#### Scenario: duplicate normalized entry is rejected

- **WHEN** a ZIP contains two entries that normalize to the same archive path
- **THEN** inspection returns a typed duplicate-name outcome and extraction
  creates no artifact BlobRef

#### Scenario: declared decompression bomb is rejected

- **WHEN** a ZIP entry's declared decompressed size or compression ratio
  exceeds its configured limit
- **THEN** inspection returns a typed limit outcome before that entry's bytes
  are decompressed

### Requirement: Extraction remains bounded and quarantines media

The service SHALL extract only a successful inspection inventory into an owned
isolated staging area, re-check byte limits while streaming decompression, and
never execute, render, or auto-trust extracted content. Every non-directory
entry SHALL be classified as text-like, structured-data, HTML-like, media, or
unknown; media and HTML-like content SHALL remain quarantined references.

#### Scenario: decompression overrun aborts without artifact publication

- **WHEN** an entry expands beyond the inspected or configured byte limit
  while extraction streams it
- **THEN** extraction fails with a typed limit outcome and publishes no BlobRef
  for that entry

#### Scenario: image stays a quarantined reference

- **WHEN** extraction accepts an image entry
- **THEN** the result records it as quarantined media and exposes only its
  immutable BlobRef and provenance, without decoding or rendering it

### Requirement: Extracted artifacts retain raw evidence provenance

The service SHALL store every accepted non-directory extracted entry through
the service BlobStore and return its BlobRef together with immutable provenance
containing the source raw archive digest and normalized entry name. The
artifact's bytes SHALL verify through its BlobRef before extraction succeeds.

#### Scenario: extracted JSON links back to the raw archive digest

- **WHEN** a verified raw ZIP with `conversations.json` is extracted
- **THEN** the extracted artifact has a verified BlobRef and provenance whose
  source archive digest equals the raw ZIP BlobRef digest
