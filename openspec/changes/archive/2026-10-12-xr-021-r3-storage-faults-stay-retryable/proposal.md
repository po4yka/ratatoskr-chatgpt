## Why

The independent review of XR-021 round 3 rejected the registered parser path that verifies `assets.json` (CONTRACTS.md R3-05): every blob-store failure became `ParserExecutionError::Failed`, and the importer treats that as a permanent failure. One transient disk or I/O fault while importing an export that contains `assets.json` therefore ran `mark_permanent_failure`, wrote the run as `failed` and reported `chatgpt.archive.invalid` with `retryable=false`. Because a duplicate upload binds to the first run of the same export and R3-04 reports a failed run to every later operation at once, those bytes became permanently un-importable. The same fault class inside asset verification was recorded as a `BlobVerificationFailed` gap and frozen into a completed or partial import. The blob store itself reported every read failure as `Missing`, so it could not tell a refused read from an absent object.

## What Changes

- `BlobStore::verify` and `BlobStore::resolve` report an object that is absent as `Missing` and any other read failure (a refused read, a device error) as `Unavailable`.
- The registered parser path separates a storage fault from a parse failure: `ParserExecutionError::Storage` is returned when storing an archive entry or verifying a declared file fails with `Unavailable` or `Stream`, and `SyntheticParserError::StorageUnavailable` carries it out of asset verification instead of an anomaly.
- The initial importer treats `ParserExecutionError::Storage` as retryable: the pass returns an error, the run stays in its stage, no failed report is queued, and the next pass imports the export.

Breaking, with no shims: `ParserExecutionError` and `SyntheticParserError` gain a variant, and callers that matched them exhaustively must handle it.

## Impact

Touches the blob store's read classification, the synthetic parser's asset verification, the parser registry's error type and the importer's permanence rule. The two open owner decisions from the review (whether R3-07 rule 2 asks the refused-receipt line to carry the operation id, and whether AGENTS.md `Completeness reporting` yields to the synthetic grammar of R3-05) are reported, not changed.
