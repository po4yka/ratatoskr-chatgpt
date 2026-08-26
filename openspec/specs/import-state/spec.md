# import-state Specification

## Purpose
Owns the durable state machine every export import walks — from receipt through hashing, storage, inspection, parsing, and reconciliation to a terminal class — so that progress survives process restarts, transitions cannot regress, and interrupted work resumes from persisted evidence instead of restarting.

## Requirements

### Requirement: The machine declares its states and legal transitions

The import state machine SHALL consist of the non-terminal states `received`, `hashed`, `stored`, `inspected`, `parsed`, `reconciled` and the terminal states `completed`, `partial`, `failed`, `duplicate`, `quarantined`. Forward progress SHALL follow `received -> hashed -> stored -> inspected -> parsed -> reconciled -> completed | partial`; `failed` SHALL be reachable from any non-terminal state; `quarantined` SHALL be terminal and reserved for safety-policy exclusion; no other transition SHALL be accepted.

#### Scenario: the happy path advances one stage at a time

- **WHEN** an import run advances through received, hashed, stored, inspected, parsed, and reconciled in order and then completes
- **THEN** each transition is accepted, the run's recorded state equals the last requested stage, and the finish time is set only when a terminal state is entered

#### Scenario: skipping stages is refused

- **WHEN** an attempt advances a run directly from `received` to `stored`
- **THEN** the transition is refused and the run remains at `received`

#### Scenario: failed is reachable from any non-terminal stage

- **WHEN** a run at `parsed` is marked failed
- **THEN** the transition is accepted, the run records `failed` as terminal, and its earlier progress stays queryable

### Requirement: Transitions are guarded and cannot regress

Every stage change SHALL be applied as a guarded update that requires the run to currently sit at the expected source state; a replayed or concurrent command carrying a stale expected state SHALL be refused and leave the recorded state unchanged.

#### Scenario: replayed transition with stale expectation changes nothing

- **WHEN** two commands race to advance the same run from `hashed` to `stored` and both claim `hashed` as the source
- **THEN** exactly one succeeds, the other is refused, and the run ends at `stored` exactly once

#### Scenario: out-of-order commands cannot regress terminal state

- **WHEN** a command arrives to advance a completed run backwards through an earlier stage
- **THEN** the transition is refused and the run remains at its terminal state

### Requirement: Terminal states are final

A run in `completed`, `partial`, `failed`, `duplicate`, or `quarantined` SHALL accept no further transition in any direction.

#### Scenario: advancing a terminal run is refused

- **WHEN** any advance is attempted against a run at `failed`
- **THEN** the transition is refused and the run remains `failed`

### Requirement: Progress persists incrementally

Each accepted stage change SHALL be durable before the next stage begins: the digest and byte length are recorded when the run reaches `hashed`, the export link is recorded when it reaches `stored`, and a crash at any point leaves the run queryable at its last completed stage.

#### Scenario: crash between hashed and stored leaves a resumable record

- **WHEN** the process dies after a run reached `hashed` and before storage completed
- **THEN** after restart the run is still queryable at `hashed` carrying its digest and byte length

### Requirement: Interrupted runs resume from persisted evidence

Resuming a non-terminal run SHALL continue from its recorded stage using surviving staging evidence: a run at `hashed` whose staging file verifies against its recorded digest proceeds to `stored` without new bytes; a run at `received` re-verifies its staging file to reach `hashed`; a run whose staging evidence is missing or fails verification SHALL end durably as `failed`; resuming a terminal run SHALL be an accepted no-op.

#### Scenario: hashed run resumes to stored without a re-upload

- **WHEN** a run sits at `hashed` with an intact staging file that hashes to its recorded digest and resume is invoked
- **THEN** the bytes are published, the export row is created, and the run reaches `stored` without any client re-transfer

#### Scenario: lost staging evidence fails the run durably

- **WHEN** resume is invoked for a non-terminal run whose staging file is absent
- **THEN** the run becomes durably `failed` and no object is published

#### Scenario: resuming a terminal run does nothing

- **WHEN** resume is invoked for a run already at a terminal state
- **THEN** the run is unchanged and the outcome already recorded by that terminal state is returned unchanged

### Requirement: Duplicate receipts terminate explicitly

When the duplicate check fires for a run that has reached `hashed`, the run SHALL enter the terminal `duplicate` state naming the pre-existing export, and no second export row SHALL be created.

#### Scenario: duplicate detection terminates the run without a new export row

- **WHEN** a run reaches `hashed` with a digest that already exists for the owning account and the pipeline continues
- **THEN** the run enters `duplicate` referencing the existing export identifier and the export row count for that digest is unchanged
