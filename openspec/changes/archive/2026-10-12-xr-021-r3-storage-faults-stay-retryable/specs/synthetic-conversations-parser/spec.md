## ADDED Requirements

### Requirement: A storage fault during parsing leaves the import retryable

A failure of the blob store while the registered parser stores an archive entry or verifies a declared file SHALL NOT fail the import permanently and SHALL NOT be recorded as an asset anomaly or a gap. The import SHALL stay in its stage with no failed report queued and SHALL complete on a later pass once storage works.

#### Scenario: Store fails during an assets import

- **WHEN** the blob store refuses writes while an export containing `assets.json` is imported
- **THEN** the pass returns an error, the run is not `failed`, no report is queued for its operation, and after the store recovers the next pass completes the import with its assets

#### Scenario: Verification hits a refused read

- **WHEN** a declared file's stored object cannot be read because the store refuses the read
- **THEN** parsing returns a storage error and no asset is recorded as `BlobVerificationFailed`
