## Why

Conversation-only reconciliation cannot preserve the files, generated assets, Canvas documents, or project instructions that an export actually contains. Those records must remain evidence-linked, digest-verified, and conservative about missing or anomalous bytes before the archive can describe project coverage truthfully.

## What Changes

- Extend the supported synthetic archive evidence grammar with optional project metadata, instructions/system prompts, Canvas-like documents, asset references, and archive-contained asset bytes.
- Produce typed, parser-stamped project, Canvas, and asset evidence while retaining unknown provider-shaped fields and never fetching references absent from the export.
- Verify an asset's extracted `BlobRef` before associating it; quarantine assets with mismatched declared evidence or unsafe extracted artifacts, and report missing or quarantined coverage without rendering or previewing bytes.
- Extend deterministic reconciliation and completeness reporting to retain project, instruction, Canvas, and asset observations/revisions alongside the existing conversation evidence.

## Capabilities

### New Capabilities

- `archive-assets-project-evidence`: Parser and reconciliation behavior for export-evidenced projects, instructions, Canvas documents, and asset references/bytes.

### Modified Capabilities

- `synthetic-conversations-parser`: The synthetic parser's accepted archive evidence and normalized result gain project, Canvas, and asset evidence.
- `archive-reconciliation`: Reconciliation retains and revisions the newly parsed evidence without absence-based deletion.
- `archive-completeness-reporting`: Completeness reports distinguish evidenced, missing, and quarantined assets and project-document coverage.

## Impact

- `ratatoskr-chatgpt-archive` parser models, synthetic fixture inputs, asset verification seam, reconciliation model, and completeness reports.
- Existing first-version schema definitions may be edited in place only if persistence is required by the approved design; no migration tooling, new major API, consumer session automation, preview, OCR, external download, or cross-repository event contract is introduced.
- The change uses the workspace `blob-references` contract for `BlobRef` semantics; raw archive bytes remain owned by this service.
