## 1. A storage fault is not a missing object (R3-04 rule 2)

- [x] 1.1 RED: `tests/blob_store.rs` `an_unreadable_object_is_unavailable_and_an_absent_one_is_missing` failed with "a refused read is a storage fault, got Err(Missing)".
- [x] 1.2 GREEN: `read_verified` and `locate` classify read failures through one rule; the absent-object half of the test stayed green.

## 2. The parser separates a storage fault from a parse failure (R3-05)

- [x] 2.1 RED: `tests/archive_asset_evidence.rs` `a_storage_fault_while_verifying_a_declared_file_is_an_error_not_an_anomaly` failed with `Ok([Some(BlobVerificationFailed)])`; `tests/import_state.rs` `a_storage_fault_during_an_assets_import_is_retried_not_failed` failed with "a storage fault is an error the pump retries, got Ok(1)" (the run was marked failed).
- [x] 2.2 GREEN: `ParserExecutionError::Storage` and `SyntheticParserError::StorageUnavailable` carry the fault out of `extract_entries` and asset verification, and the importer's permanence rule names only `ParserExecutionError::Failed`.

## 3. Gate

- [x] 3.1 The full gate passed locally against a private PostgreSQL 17 and JetStream broker: `cargo fmt --all -- --check`, clippy with `-D warnings`, the 850-line check, `cargo build` and `cargo test --workspace --locked`, `cargo deny check`, `openspec validate --all --strict` and `openspec validate --archived`.
