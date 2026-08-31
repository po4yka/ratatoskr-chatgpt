## ADDED Requirements

### Requirement: Raw receipt durably schedules non-terminal import work

A Platform-bound receipt SHALL atomically associate the operation with verified immutable raw
evidence and durable import work. Raw storage alone SHALL NOT enqueue a terminal operation report.

#### Scenario: Process stops after raw persistence

- **WHEN** the process stops after publishing verified raw bytes but before parsing them
- **THEN** no terminal operation report exists and restart discovers the same pending import work

### Requirement: Duplicate bytes retain every Platform operation correlation

Receiving a digest already owned by the account SHALL reuse the existing raw archive and normalized
import result while durably associating the newly supplied Platform operation.

#### Scenario: Equal bytes arrive for another operation

- **WHEN** identical verified bytes arrive under a second Platform operation identifier
- **THEN** one raw archive remains and both operations are eligible for their own truthful result
