## 1. Archive-evidence parser boundary and synthetic fixtures

- [x] 1.1 Add the compiling public `SyntheticArchiveInput`, parsed project/Canvas/asset record seams, and invented archive fixtures (including one safe text asset) required for integration tests; verify the pre-existing crate suite remains green. This API/fixture prerequisite cannot start with a failing behavioral test because the public input and fixture identifiers must compile before a test can call them.
- [x] 1.2 Add `crates/chatgpt-archive/tests/archive_asset_evidence.rs` with `project_and_instruction_evidence_is_preserved` and `canvas_document_content_is_preserved_as_evidence`; run `build-gate -- cargo nextest run --locked -p ratatoskr-chatgpt-archive --test archive_asset_evidence project_and_instruction_evidence_is_preserved` and confirm the project/instruction assertion fails because the conservative seam has no parsed project evidence, not because the test fails to compile.
- [x] 1.3 Replace the byte-only parse boundary per design D1 and implement typed, ordered project/instruction/Canvas decoding plus raw-field preservation; rerun both exact tests from 1.2 and confirm they pass without rendering, executing, or fetching any referenced content.

## 2. Verified asset association and quarantine

- [x] 2.1 Add `asset_digest_mismatch_is_quarantined` to `crates/chatgpt-archive/tests/archive_asset_evidence.rs`; run it through `build-gate -- cargo nextest run --locked -p ratatoskr-chatgpt-archive --test archive_asset_evidence asset_digest_mismatch_is_quarantined` and confirm its expected quarantine assertion fails because the seam has not verified asset declarations.
- [x] 2.2 Implement the `AssetVerifier` seam and exact extracted-artifact matching: verify candidate BlobRefs, compare SHA-256/length/media type, and retain mismatches, missing candidates, and pre-quarantined artifacts as non-usable quarantined evidence; rerun the exact mismatch test and confirm it passes.
- [x] 2.3 Add `verified_asset_keeps_its_blob_reference` and `reference_only_asset_remains_missing` to `crates/chatgpt-archive/tests/archive_asset_evidence.rs`; run each through the gate and confirm the BlobRef/availability assertions fail because positive and reference-only asset states are not yet represented.
- [x] 2.4 Implement verified and missing asset states, uploaded/generated provenance, owner relationships, and deterministic asset ordering without reopening archive paths or fetching URLs; rerun both tests from 2.3 and confirm they pass.

## 3. Append-only evidence reconciliation

- [x] 3.1 Add `project_and_asset_revisions_are_append_only` to `crates/chatgpt-archive/tests/archive_reconciliation.rs`; run it through `build-gate -- cargo nextest run --locked -p ratatoskr-chatgpt-archive --test archive_reconciliation project_and_asset_revisions_are_append_only` and confirm the revision-chain assertion fails because reconciliation does not yet retain project or asset evidence.
- [x] 3.2 Generalize deterministic canonical digest/revision histories for projects, instructions, Canvas documents, and assets, including asset availability/BlobRef as revision evidence; rerun the exact revision test and confirm it passes.
- [x] 3.3 Add `missing_project_evidence_is_an_observation_not_a_deletion` to `crates/chatgpt-archive/tests/archive_reconciliation.rs`; run it through the gate and confirm the expected missing-observation assertion fails because project absence is not yet reconciled.
- [x] 3.4 Implement non-destructive missing-from-latest observations for the new evidence kinds while retaining every prior revision and avoiding inferred deletion, lost access, or asset availability; rerun the exact missing-project test and confirm it passes.

## 4. Conservative completeness reporting

- [x] 4.1 Add `quarantined_asset_keeps_completeness_partial` to `crates/chatgpt-archive/tests/archive_reconciliation.rs`; run it through `build-gate -- cargo nextest run --locked -p ratatoskr-chatgpt-archive --test archive_reconciliation quarantined_asset_keeps_completeness_partial` and confirm the private-count/partial-class assertion fails because reports do not yet count the new evidence.
- [x] 4.2 Extend archive-local and cumulative reports with structured project/instruction/Canvas/asset counts, unobserved/missing/quarantined gaps, deterministic ordering, and no private source values; rerun the exact reporting test and confirm it passes.

## 5. Documentation, full validation, and delivery

- [x] 5.1 Update `README.md` and affected crate documentation to describe the synthetic-only project/Canvas/asset evidence boundary, BlobRef verification, quarantine semantics, and no-preview/no-OCR limits; verify `git diff --check`. Documentation has no direct failing-test precondition.
- [x] 5.2 Run `git diff --check`, `openspec validate --all --strict`, `cargo fetch --locked`, `cargo deny check`, `cargo fmt --all -- --check`, `build-gate -- cargo clippy --workspace --all-targets --locked -- -D warnings`, the tracked-Rust file-length check, `build-gate -- cargo build --workspace --locked`, and `build-gate -- cargo test --workspace --locked`; inspect the final diff and tick only completed tasks.
- [x] 5.3 Archive `archive-assets-canvas-project-evidence`, verify `openspec validate --archived`, commit the dedicated worktree branch, fast-forward `main`, push `main`, and remove only this task worktree and branch after the pushed remote state is observed. This delivery task has no failing unit test because it publishes already-validated work.
