## ADDED Requirements

### Requirement: Blob erasure and blob registration share one lock

The eraser SHALL hold the digest lock for the reference re-check and the byte erase of each blob in one transaction. A registrar SHALL take the same digest lock before inserting the first row that references a digest and SHALL verify the blob is present. If the blob is missing the registrar SHALL re-store it from the bytes it still holds, and otherwise SHALL fail with a retryable class and register nothing. At no commit point SHALL a row reference a missing blob.

#### Scenario: Import of identical bytes while the eraser holds the lock

- **WHEN** the eraser holds the digest lock after its re-check and another tenant's import registers a reference to the same digest
- **THEN** the import blocks until the eraser commits, finds the blob missing, re-stores it from its staged bytes, and the committed state is a row that references an existing blob

#### Scenario: Import commits first

- **WHEN** the import registers its reference before the eraser takes the lock
- **THEN** the eraser's re-check finds the reference and keeps the blob as `retain_shared`
