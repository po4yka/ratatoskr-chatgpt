# Tasks: authenticated-archive-receipt

## 1. Import state machine core

- [x] 1.1 Add `crates/chatgpt-archive/tests/import_state.rs` with `happy_path_advances_stage_by_stage_and_finishes_on_terminal`, `skipping_a_stage_is_refused`, `failed_is_reachable_from_every_non_terminal_state`, `terminal_states_accept_no_transition`, and `guarded_advance_rejects_a_stale_source`. Run `cargo test --test import_state` and confirm all fail because the receipt module does not exist.
- [x] 1.2 Implement `src/receipt/state.rs`: `ImportState` (received, hashed, stored, inspected, parsed, reconciled; terminal completed, partial, failed, duplicate, quarantined), pure transition validation returning the target or a typed refusal; verify tests from 1.1 pass.

## 2. Configuration keys

- [x] 2.1 Extend `crates/chatgpt-archive/tests/config.rs` with `max_archive_bytes_parses_with_documented_default`, `non_positive_archive_cap_is_reported_value_free`, `relative_staging_root_is_refused`, `tenant_tokens_parse_into_secret_pairs`, `malformed_tenant_token_entries_are_each_reported`, and `tenant_tokens_render_redacted_in_debug`. Confirm each fails against current config before implementing.
- [x] 2.2 Extend `src/config.rs`: `limits.max_archive_bytes` (default 17_179_869_184 = 16 GiB), `storage.receipt_staging_root: Option<PathBuf>`, `receipt.tenant_tokens: Vec<(SecretString, String)>` parsed from comma-separated `<token>=<external-ref>` pairs with one violation per malformed entry; secret redaction in Debug/Serialize. Verify tests from 2.1 pass.

## 3. Authentication seam

- [x] 3.1 Add `crates/chatgpt-archive/tests/receipt_auth.rs` with `configured_token_resolves_its_tenant_account`, `missing_credential_is_unauthenticated`, `unknown_token_is_indistinguishable_from_missing`, and `malformed_authorization_header_is_unauthenticated`. Confirm they fail.
- [x] 3.2 Implement `src/receipt/auth.rs`: `TenantPrincipal`, `TenantAuthenticator` trait, and the config-token-map implementation that resolves a bearer credential to its account external ref; missing/malformed/unknown produce one identical unauthenticated outcome. Verify tests from 3.1 pass.

## 4. Schema edited in place

- [x] 4.1 Extend `crates/chatgpt-archive/tests/persistence_schema.rs` with `equal_digests_coexist_across_accounts_only`, `export_row_requires_an_owner_account`, and `run_exists_before_export_and_rejects_unknown_states`. Run against `CHATGPT_TEST_DATABASE_URL` and confirm they fail because the schema still carries the bootstrap shape.
- [x] 4.2 Edit `schema.sql` in place per design D4: exports gain NOT NULL account and per-account digest uniqueness; import_runs gains nullable export link, digest/byte-length columns, and the import-state CHECK set. Verify tests from 4.1 pass and both pre-existing persistence tests stay green.

## 5. Receiver pipeline

- [x] 5.1 Add `crates/chatgpt-archive/tests/receipt_receiver.rs` plus a hand-written `FakeReceiptRepository` behind `test-support`, with `chunked_upload_hashes_incrementally_and_records_verified_evidence`, `declared_length_over_cap_is_refused_before_reading`, `streamed_overrun_aborts_at_cap_and_fails_run_durably`, `truncated_stream_publishes_nothing_but_fails_run_durably`, `identical_reupload_answers_duplicate_without_new_rows`, `different_content_stores_as_new_export`, and `stored_bytes_verify_against_the_recorded_digest`. Confirm they fail.
- [x] 5.2 Implement `src/receipt/mod.rs`: `ArchiveReceiver` with tee-to-staging streaming hash (D1/D2), cap enforcement, duplicate check, BlobStore publish from staged bytes, export recording, bounded telemetry counters, and the `ReceiptRepository` trait seam consumed through fakes. Verify tests from 5.1 pass.
- [x] 5.3 Add receiver resume tests: `hashed_run_resumes_to_stored_without_new_bytes`, `received_run_reverifies_staging_then_advances`, `missing_staging_fails_the_run_durably`, `resuming_a_terminal_run_changes_nothing`. Confirm they fail.
- [x] 5.4 Implement resume on `ArchiveReceiver` per design D9 semantics. Verify tests from 5.3 and the whole crate suite pass.

## 6. Postgres repository

- [x] 6.1 Add `crates/chatgpt-archive/tests/receipt_repository_pg.rs` (skipped without `CHATGPT_TEST_DATABASE_URL`) with `run_roundtrips_through_create_load_and_advance`, `guarded_transition_accepts_once_then_refuses_stale_source`, `duplicate_lookup_scopes_by_account`, and `record_export_persists_reference_digest_and_timestamps`. Confirm they fail.
- [x] 6.2 Implement `src/receipt/repository.rs`: `PostgresReceiptRepository` over runtime-checked sqlx queries, including the account upsert on first receipt. Verify tests from 6.1 pass.

## 7. HTTP surface

- [x] 7.1 Add `crates/chatgpt-archive/tests/receipt_http.rs` with `stored_receipt_answers_201_with_identity`, `duplicate_receipt_answers_200_naming_the_existing_export`, `unauthenticated_requests_render_the_401_envelope`, `oversized_declaration_renders_the_413_envelope`, `wrong_method_answers_405`, and `missing_acquisition_or_media_type_answer_400`. Confirm they fail.
- [x] 7.2 Implement `src/receipt/http.rs` (router + handler over `ArchiveReceiver`), add `FailureKind::PayloadTooLarge` to `src/error.rs` with its inventory entry, export the module tree from `lib.rs`. Verify tests from 7.1 pass.

## 8. Service wiring

- [x] 8.1 Extend `services/chatgpt-archive/tests/boot.rs` with `receipt_route_serves_end_to_end_when_configured` (spawns the binary with staging root, tenant tokens, blob root, and `CHATGPT_TEST_DATABASE_URL`; uploads a synthetic archive twice asserting stored then duplicate outcomes). Skipped without the database URL like persistence tests; confirm it fails while wiring is absent.
- [x] 8.2 Wire `services/chatgpt-archive/src/lib.rs`: build receiver state when staging is configured, mount the public router beside admin, run the startup resume sweep best-effort after schema application. Minimal-environment boot test must stay green. Verify tests from 8.1 pass.

## 9. Documentation truthfulness

- [x] 9.1 Update README status block and the local-run snippet (new env keys) plus the crate-level doc in `lib.rs` to state that authenticated receipt, streaming hashing, immutable raw storage, and the durable import state exist while parsing does not. Documentation only: no failing test can express prose accuracy.
- [x] 9.2 Record the token-map stopgap limitation note (design D5) in README's security section pointer. Documentation only.

## 10. Gate and archive

- [x] 10.1 Run the full gate: `git diff --check`, `openspec validate --all --strict`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, file-length check, `cargo deny check`, `cargo fetch --locked`, `cargo build --workspace --locked`, `cargo test --workspace --locked` with `CHATGPT_TEST_DATABASE_URL` set, then tick every task above.
- [x] 10.2 Archive the change (`openspec archive authenticated-archive-receipt`) and confirm `openspec validate --archived` passes.
