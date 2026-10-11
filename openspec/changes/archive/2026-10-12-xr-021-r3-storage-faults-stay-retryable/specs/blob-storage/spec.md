## ADDED Requirements

### Requirement: A storage fault is distinct from a missing object

Verifying or resolving a `BlobRef` SHALL report an object that does not exist, or does not verify, as missing, and SHALL report any other failure to read it as an unavailable store, so a caller can retry the latter without concluding anything about the object.

#### Scenario: Unreadable object

- **WHEN** an object exists but its directory refuses reads and the matching reference is verified
- **THEN** verification fails as unavailable, not as missing

#### Scenario: Absent object

- **WHEN** the object of a stored reference is removed and the reference is verified
- **THEN** verification fails as missing
