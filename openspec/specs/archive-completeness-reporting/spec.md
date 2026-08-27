# archive-completeness-reporting Specification

## Purpose

Produces conservative, deterministic archive-local and cumulative evidence
reports so an archive owner can assess coverage without optimistic claims.

## Requirements

### Requirement: Every reconciled archive has an evidence-based report

For every reconciled archive snapshot, the service SHALL produce one report
containing the archive identity, parser/schema identity, discovered
conversation and message counts, graph-orphan count, parse and graph warnings,
coverage gaps, new/reused revision counts, and missing-conversation observation
count. The report SHALL contain only structured identifiers and counts; it
SHALL NOT include titles, message bodies, filenames, or raw provider values.

#### Scenario: archive report totals match its snapshot evidence

- **WHEN** `per_archive_report_counts_fixture_evidence` reconciles a fixture
  export containing known conversation/message totals and one orphan
- **THEN** its report exposes exactly those totals, one orphan warning, and the
  corresponding revision statistics without source content

### Requirement: Cumulative report aggregates append-only reconciliation evidence

The service SHALL produce a cumulative report over all reconciled snapshots.
Its totals SHALL distinguish unique identities from revision count and archive
observations, and SHALL sum archive-local warnings and missing observations.
The report SHALL classify completeness conservatively: it SHALL NOT report
`Complete` while project relationship or asset coverage is unobserved.

#### Scenario: cumulative report arithmetic remains conservative

- **WHEN** `cumulative_report_sums_revisions_gaps_and_warnings` reconciles a
  fixture sequence with one changed conversation, one omitted conversation,
  and an unobserved project relationship
- **THEN** the report distinguishes unique conversations from revisions,
  includes the missing observation and warnings, and classifies coverage as
  structurally partial rather than complete

### Requirement: Report ordering is deterministic for identical archive sequences

For identical ordered input snapshots, report records, warnings, gaps, and
revision statistics SHALL be returned in the same order and with equal values
on every reconciliation run.

#### Scenario: identical sequences yield identical reports

- **WHEN** `reconciliation_reports_are_deterministic` reconciles the same
  fixture sequence twice
- **THEN** both archive-local reports and the cumulative report are equal
