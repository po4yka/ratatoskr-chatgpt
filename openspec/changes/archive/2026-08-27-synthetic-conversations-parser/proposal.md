## Why

Safe intake and deterministic registry selection now retain `conversations.json`,
but the service has no parser that can turn a known synthetic export structure
into lossless normalized conversation evidence. A narrow first parser establishes
the parser contract and deterministic records before graph reconciliation and
asset archival add their own semantics.

## What Changes

- Add a versioned parser for a documented synthetic `conversations.json` shape
  selected through the existing registry for consumer exports.
- Normalize conversations, messages, ordered content parts, timestamps, and
  model/slug metadata into records shaped for the existing `chatgpt_archive`
  schema without persisting or reconciling them across snapshots.
- Preserve every unrecognized field and content-part variant as raw JSON beside
  its normalized record; do not fetch, store, or claim ownership of asset bytes.
- Add a committed synthetic fixture and deterministic mapping tests, including
  parser-version stamps and unknown-field preservation.
- Add a private, owner-authorized real-export golden validation path. It remains
  blocked until the owner supplies a redacted fixture; no personal export is
  committed to this repository.

## Capabilities

### New Capabilities

- `synthetic-conversations-parser`: Parses the documented synthetic
  `conversations.json` structure into deterministic, loss-aware normalized
  conversation, message, and content-part records.

### Modified Capabilities

- None.

## Impact

- `crates/chatgpt-archive`: parser types and implementation, registry wiring,
  and public parser result contract.
- `crates/chatgpt-archive/tests`: committed synthetic fixture and mapping,
  determinism, parser-stamp, and unknown-preservation tests.
- No database migration, database write path, archive reconciliation, asset
  archival, portable export, cross-repository event contract, browser
  automation, or inference integration.
