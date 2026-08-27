## Why

Platform now creates an archive operation before streaming bytes, but ChatGPT's receipt accepts
only its legacy tenant bearer and never receives the Platform operation identifier. A completed raw
archive therefore cannot advance the operation the export agent polls.

## What Changes

- Add a loopback-only, Edge-claim receipt endpoint for a prepared ChatGPT archive. It accepts no
  caller bearer and refuses missing/malformed minted user, device, correlation, operation, digest,
  or length claims.
- Resolve the minted Platform user through an explicit local account mapping; do not infer a
  ChatGPT account from an arbitrary UUID or tenant string.
- Verify the streamed bytes against Edge's declared digest and size through the existing raw-first
  receiver, then publish one terminal `platform.operation.reported.v1` event using the existing
  bounded AI archive summary contract.

## Impact

Touches the ChatGPT receipt HTTP surface, account configuration/persistence mapping, event outbox,
and terminal import projection. Platform remains the owner of the public operation; ChatGPT remains
the owner of raw bytes, parsing and completeness.
