## Why

Archive ownership is incomplete if a tenant cannot take a readable, independently
verifiable copy of the normalized evidence and locally verified assets. The current
archive projections have no portable export boundary.

## What Changes

- Add a tenant-scoped portable archive export command that writes a deterministic ZIP.
- Include canonical normalized JSON, readable Markdown for projects and conversations,
  verified asset bytes, and a manifest with SHA-256 digests and raw-archive/parser
  provenance.
- Support deterministic project and inclusive date filtering without treating absence
  as deletion.
- Fail closed if an asset claimed for export cannot be read and verified against its
  `BlobRef`; retain its availability and warning evidence in the manifest.

## Capabilities

### New Capabilities

- `portable-archive-export`: Produce a tenant-scoped, deterministic, verifiable local
  archive from normalized ChatGPT archive evidence.

### Modified Capabilities

- None.

## Impact

- Affects the Rust domain library and service binary command surface.
- Uses the existing BlobStore and `BlobRef` fleet contract without adding a shared
  blob service or a schema migration.
- Adds synthetic fixtures and golden-contract coverage; no provider import or UI is
  introduced.
