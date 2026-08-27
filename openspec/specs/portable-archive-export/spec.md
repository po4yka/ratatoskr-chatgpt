# portable-archive-export Specification

## Purpose

Lets a tenant take a deterministic, independently verifiable local copy of
their normalized ChatGPT archive evidence and every verified asset byte.

## Requirements

### Requirement: Tenant-scoped archive state is exportable

The service SHALL persist normalized archive snapshots with their owning account,
source export identity, raw-export digest, parser identity, and observed time,
and SHALL load only evidence owned by the requested tenant for portable export.
It SHALL NOT return another tenant's projects, conversations, raw provenance, or
asset references.

#### Scenario: tenant selection excludes another account

- **WHEN** `tenant_scope_excludes_other_account_evidence` exports a state containing
  projects and conversations for two accounts
- **THEN** the produced archive contains only records and provenance belonging to
  the requested account

### Requirement: Portable output is deterministic and complete

For identical selected archive state, the export command SHALL produce byte-identical
ZIP output. It SHALL contain a manifest, canonical normalized JSON, readable
Markdown renderings, and every verified selected asset. The manifest SHALL list every
written member in lexicographic path order with SHA-256 digest, byte length, media
type when available, and source-export/parser provenance; it SHALL also retain missing
or quarantined asset availability as warnings without fabricating bytes.

#### Scenario: identical state has a stable archive digest

- **WHEN** `identical_state_produces_byte_identical_zip` exports the same selected
  state twice
- **THEN** the two ZIP byte sequences and every manifest member digest are equal

#### Scenario: manifest describes every exported member

- **WHEN** `manifest_lists_json_markdown_and_verified_asset_members` exports a project,
  a conversation, and a verified asset
- **THEN** its manifest lists exactly the generated JSON, Markdown, and asset paths with
  matching digests and provenance, and no asset byte is substituted for an unavailable asset

### Requirement: Project and date filters restrict selected evidence

The export command SHALL require an account scope and SHALL support an optional exact
project filter and inclusive observed-time range. It SHALL apply all supplied filters
before rendering or copying assets, preserve deterministic ordering after filtering,
and report the applied filters in the manifest.

#### Scenario: project and time filters select only matching evidence

- **WHEN** `filters_limit_export_to_matching_project_and_observed_time` exports a
  tenant state containing records inside and outside an inclusive time range and project
- **THEN** the archive contains only evidence matching both filters and the manifest
  records the requested filter values

### Requirement: Portable output retains provenance without changing raw evidence

Each normalized JSON record and Markdown rendering SHALL begin with a provenance header
that names the source archive digest and parser name/version without exposing credentials.
The export command SHALL read verified asset bytes through the owner service's `BlobRef`
contract and SHALL fail without writing a successful archive when a claimed verified
asset cannot be verified.

#### Scenario: unreadable claimed asset aborts export

- **WHEN** `unreadable_verified_asset_aborts_without_archive` exports a selected asset
  marked verified whose `BlobRef` cannot be read and verified
- **THEN** the command reports an export failure and leaves no completed output archive
