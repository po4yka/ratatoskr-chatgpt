## MODIFIED Requirements

### Requirement: Every reconciled archive has an evidence-based report

For every reconciled archive snapshot, the service SHALL produce one report
containing the archive identity, parser/schema identity, discovered conversation
and message counts, graph-orphan count, project, instruction, Canvas-document,
asset-reference, verified-asset, missing-asset, and quarantined-asset counts,
parse and graph warnings, coverage gaps, new/reused revision counts, and
missing-record observation count. The report SHALL contain only structured
identifiers and counts; it SHALL NOT include titles, message bodies, filenames,
instructions, document content, raw provider values, or asset digests.

#### Scenario: archive report totals match its snapshot evidence

- **WHEN** `per_archive_report_counts_fixture_evidence` reconciles a fixture
  export containing known conversation/message totals and one orphan
- **THEN** its report exposes exactly those totals, one orphan warning, and the
  corresponding revision statistics without source content

#### Scenario: asset evidence counts remain private and exact

- **WHEN** `quarantined_asset_keeps_completeness_partial` reconciles a fixture
  export containing one project and one quarantined asset
- **THEN** its report exposes the project and quarantined-asset counts with a
  partial class without exposing asset or project content

### Requirement: Cumulative report aggregates append-only reconciliation evidence

The service SHALL produce a cumulative report over all reconciled snapshots.
Its totals SHALL distinguish unique identities from revision count and archive
observations, and SHALL sum archive-local warnings and missing observations. The
report SHALL classify completeness conservatively: it SHALL NOT report
`Complete` while project relationship, Canvas, or asset coverage is unobserved,
or while an observed asset is missing or quarantined.

#### Scenario: cumulative report arithmetic remains conservative

- **WHEN** `cumulative_report_sums_revisions_gaps_and_warnings` reconciles a
  fixture sequence with one changed conversation, one omitted conversation,
  and an unobserved project relationship
- **THEN** the report distinguishes unique conversations from revisions,
  includes the missing observation and warnings, and classifies coverage as
  structurally partial rather than complete

#### Scenario: asset anomaly prevents complete classification

- **WHEN** `quarantined_asset_keeps_completeness_partial` reconciles a
  snapshot with a quarantined asset
- **THEN** the cumulative report retains the anomaly count and does not classify
  coverage as complete
