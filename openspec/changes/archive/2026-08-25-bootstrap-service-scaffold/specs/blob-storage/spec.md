## Purpose

Stores archive bytes immutably under content-addressed paths owned by this service and addresses them by fleet-standard blob references.

This capability cites the stored-bytes contract `blob-references` in the `ratatoskr-workspace` store for the cross-repository reference semantics.

## ADDED Requirements

### Requirement: Storing bytes yields a deterministic reference

Storing a byte stream SHALL compute the SHA-256 digest while receiving the bytes and return a `BlobRef` naming owner `ratatoskr-chatgpt`, the digest with its algorithm, the caller-declared media type, and the byte length. Storing identical bytes again SHALL produce an equal reference without rewriting the stored object.

#### Scenario: identical bytes store to the same reference

- **WHEN** the same byte sequence is stored twice through the adapter
- **THEN** both calls return references with equal digest, length, and media type, and exactly one stored object exists on disk for them

#### Scenario: digest describes the bytes

- **WHEN** bytes are stored and the stored object is re-read and hashed independently
- **THEN** the computed SHA-256 equals the digest carried by the returned reference

### Requirement: Stored objects are immutable

Once published under its digest, a stored object SHALL never be modified or replaced by subsequent operations; publishing uses staging plus an atomic publish step so a partial write can never appear under a final path.

#### Scenario: interrupted write leaves no final object

- **WHEN** a store operation fails midway through streaming
- **THEN** no object exists under the final content-addressed path for those bytes, and a later successful store of the same bytes succeeds

### Requirement: Resolution verifies before returning

Resolving a `BlobRef` SHALL return a readable path only when an object exists whose owner, algorithm, digest, length, and media type all match the reference; any mismatch SHALL be reported as a missing artifact rather than as different content.

#### Scenario: mismatching object reads as missing

- **WHEN** a stored object's bytes are corrupted on disk and the matching reference is resolved with verification
- **THEN** resolution fails as missing rather than returning a path to the corrupted bytes

#### Scenario: foreign owner cannot resolve

- **WHEN** a `BlobRef` naming another service as owner is resolved against this adapter's root
- **THEN** resolution fails as missing without reading any object

### Requirement: The storage backend is replaceable

The adapter SHALL separate what storing means (digest, reference, immutability) from where bytes are kept behind an internal seam, so a remote object-store backend can be added without changing callers or the reference format.

#### Scenario: local backend serves the contract unchanged

- **WHEN** the adapter is exercised against its filesystem backend by the store/resolve/verify tests
- **THEN** all behavior scenarios pass against that backend alone, with no test reaching into filesystem layout specifics
