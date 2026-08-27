## 1. Testable reconciliation boundary

- [x] 1.1 Add compiling public reconciliation types, an explicit conservative
  unsupported seam, and a minimized ordered fixture-sequence helper in
  `crates/chatgpt-archive/tests/archive_reconciliation.rs`; verify the
  existing archive-crate suite remains green. This API/fixture prerequisite
  cannot start with a failing behavioral test because the public types must
  compile before an external test can invoke the seam.

## 2. Append-only revisions

- [x] 2.1 Add `revision_chain_builds_across_fixture_exports` to
  `crates/chatgpt-archive/tests/archive_reconciliation.rs`; run it and confirm
  its expected assertion fails because the conservative seam produces no
  revision chain, not because the test fails to compile.
- [x] 2.2 Implement deterministic conversation/message identity and digest
  revision chains plus present observations in the reconciliation module; rerun
  `cargo nextest run --locked -p ratatoskr-chatgpt-archive --test archive_reconciliation revision_chain_builds_across_fixture_exports`
  and confirm it passes.

## 3. Conservative absence and graph evidence

- [x] 3.1 Add
  `missing_conversation_becomes_observation_not_deletion` to
  `crates/chatgpt-archive/tests/archive_reconciliation.rs`; run it and confirm
  its expected assertion fails because no missing observation is emitted by the
  conservative seam, not because the test fails to compile.
- [x] 3.2 Implement `MissingFromLatestSnapshot` observations for previously
  seen, later-omitted conversations while retaining earlier revisions; rerun
  the exact missing-conversation test and confirm it passes.
- [x] 3.3 Add `orphan_parent_is_retained_and_reported` and
  `conversation_only_snapshot_reports_project_relationship_gap` to
  `crates/chatgpt-archive/tests/archive_reconciliation.rs`; run them and
  confirm their assertions fail because the seam reports neither graph evidence
  nor coverage gaps, not because the tests fail to compile.
- [x] 3.4 Implement message-parent validation, retained orphan observations,
  structured non-content warning codes, and explicit unobserved project
  relationship gaps; rerun both exact tests and confirm they pass.

## 4. Completeness reports

- [x] 4.1 Add `per_archive_report_counts_fixture_evidence`,
  `cumulative_report_sums_revisions_gaps_and_warnings`, and
  `reconciliation_reports_are_deterministic` to
  `crates/chatgpt-archive/tests/archive_reconciliation.rs`; run them and
  confirm their report-statistic assertions fail because the seam has no
  reports, not because the tests fail to compile.
- [x] 4.2 Implement deterministic per-archive and cumulative completeness
  reports, including coverage classification, counts, gaps, warnings,
  revisions, and observations; rerun the exact report tests and confirm they
  pass.

## 5. Documentation, validation, and delivery

- [x] 5.1 Update `README.md` and any affected Rust documentation to describe
  reconciliation/report scope and explicit limitations; verify
  `git diff --check` and that documentation does not claim project or asset
  coverage.
- [x] 5.2 Run `git diff --check`, `openspec validate --all --strict`,
  `cargo fetch --locked`, `cargo deny check`, `cargo fmt --all -- --check`,
  `build-gate -- cargo clippy --workspace --all-targets --locked -- -D warnings`,
  the tracked-Rust file-length check, `build-gate -- cargo build --workspace --locked`,
  and `build-gate -- cargo test --workspace --locked`; inspect the final diff
  and tick only the completed tasks.
- [x] 5.3 Archive `reconcile-archive-revisions` with OpenSpec, verify
  `openspec validate --archived`, commit the dedicated worktree branch,
  fast-forward `main`, push `main`, and remove only this task worktree and
  branch after the push is observed. This delivery task has no failing unit
  test because it archives and publishes already-validated work.
