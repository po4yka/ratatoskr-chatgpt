## Execution correction

The provenance and registry acceptance tests (3.1 and 4.2) were present when
this continuation began, so their original RED runs cannot be reconstructed
honestly. Their green runs are recorded below and the implementation was
strengthened with an independently observed RED-to-GREEN regression:
`type_detection_does_not_trust_a_media_extension`. Future behavior changes
remain subject to the test-first pairs above.

## 1. Dependency and limits

- [x] 1.1 Add exact `zip` reader dependency with default features disabled and run `cargo metadata` to create a candidate lockfile; inspect every new lockfile package and confirm `cargo deny --locked check bans advisories sources` is green. This configuration task cannot begin with a failing behavior test because it only makes the approved ZIP reader available.
- [x] 1.2 Add `MAX_ARCHIVE_ENTRIES`, `MAX_ARCHIVE_ENTRY_BYTES`, `MAX_ARCHIVE_DECOMPRESSED_BYTES`, and `MAX_ARCHIVE_COMPRESSION_RATIO` tests to `crates/chatgpt-archive/tests/config.rs`, including `non_positive_extraction_caps_are_value_free`; run it and confirm the new assertion fails because the keys are unknown.
- [x] 1.3 Extend `config::Limits` and the closed environment parser with positive documented defaults and value-free violations; rerun `cargo test --locked -p ratatoskr-chatgpt-archive --test config` and confirm it passes.

## 2. Inspector hostile-input boundary

- [x] 2.1 Add the compiling public archive-intake type seam with a conservative unsupported result, needed so the acceptance tests can execute a behavioral RED rather than fail to compile; verify the existing crate suite remains green. This is an API-shape prerequisite and implements no archive behavior.
- [x] 2.2 Add `crates/chatgpt-archive/tests/archive_intake.rs` with synthetic ZIP helpers and `inspector_lists_structure_with_bounded_safe_type_detection`, `zip_slip_is_rejected_before_extraction`, `traversal_is_rejected_before_extraction`, `duplicate_normalized_names_are_rejected`, and `declared_bomb_is_rejected_before_decompression`; run the test target and confirm each new assertion fails against the conservative seam.
- [x] 2.3 Implement central-directory inspection, archive-path normalization, duplicate/special-entry rejection, checked size accounting, compression-ratio enforcement, and structural type detection; rerun `cargo test --locked -p ratatoskr-chatgpt-archive --test archive_intake` and confirm the hostile archive suite passes.

## 3. Bounded extraction and provenance

- [x] 3.1 Extend `crates/chatgpt-archive/tests/archive_intake.rs` with `extracted_artifact_has_verified_blobref_and_raw_digest_provenance` and `media_is_quarantined_reference`; run the target and confirm the provenance assertion fails because extraction returns no artifacts.
- [x] 3.2 Implement isolated UUID staging, actual-byte rechecks during streaming decompression, BlobStore publication and verification, cleanup of only owned staging files, artifact classification, quarantine flags, and raw-digest provenance; rerun the archive-intake target and confirm all tests pass.

## 4. Versioned parser registry

- [x] 4.1 Add the compiling registry identity and selection seam with an empty registry so selection-matrix tests can run; verify the existing crate suite remains green. This API-shape prerequisite implements no parser selection behavior.
- [x] 4.2 Add `crates/chatgpt-archive/tests/parser_registry.rs` with `matching_structure_selects_one_versioned_parser`, `unsupported_structure_is_explicit`, `overlapping_capabilities_are_ambiguous`, and `duplicate_identity_is_refused`; run it and confirm each expected result fails against the empty registry.
- [x] 4.3 Implement write-once parser registration and capability matching over inspected structure plus acquisition mode, returning selected, unsupported, or ambiguous outcomes without invoking a parser; rerun `cargo test --locked -p ratatoskr-chatgpt-archive --test parser_registry` and confirm the matrix passes.

## 5. Gate and archive

- [x] 5.1 Run `git diff --check`, `openspec validate --all --strict`, `openspec validate --archived`, `cargo fetch --locked`, `cargo deny check`, `cargo fmt --all -- --check`, `build-gate -- cargo clippy --workspace --all-targets --locked -- -D warnings`, the tracked-Rust file-length check, `build-gate -- cargo build --workspace --locked`, and `build-gate -- cargo test --workspace --locked`; inspect the final diff and tick every completed task.
- [x] 5.2 Archive `safe-inspector-parser-registry` with OpenSpec and confirm `openspec validate --archived` passes; commit the worktree branch, fast-forward `main`, push `main`, and remove only this task worktree and branch after the push is observed. This delivery task has no failing unit test because it archives and publishes already-validated work.
