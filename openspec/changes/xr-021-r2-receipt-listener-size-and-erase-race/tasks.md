## 1. Contracts pin

- [x] 1.1 Configuration, so no failing test first: move the five `ratatoskr-*` git dependencies to contracts commit 5426cf3096733d714c59b02ec41e946fdedcd94d and refresh `Cargo.lock`; the existing suite stays green.

## 2. Dedicated receipt listener (R2-01)

- [x] 2.1 RED: `crates/chatgpt-archive/tests/config.rs` `receipt_listener_defaults_and_validation` fails because the key is not recognized; `services/chatgpt-archive/tests/boot.rs` `receipt_and_operator_planes_are_separate_listeners` fails because nothing listens on the receipt port (the configuration parsing was added first so the boot test could get past a refused key). `no_receipt_listener_is_bound_without_a_database_and_a_staging_root` pins a negative that already held, so it passed in the RED run.
- [x] 2.2 GREEN: `ReceiptConfig.listen_address` with validation, a separate receipt listener in `run`, both listeners in graceful shutdown, and the documentation that said the receipt is on the admin router.

## 3. Exact media type on the Platform receipt (R2-15)

- [x] 3.1 RED: `crates/chatgpt-archive/tests/receipt_http.rs` `platform_receipt_rejects_any_content_type_but_application_zip` fails for `application/octet-stream`, `application/zip; charset=binary` and `APPLICATION/ZIP` because they are accepted.
- [x] 3.2 GREEN: compare the header value with `ARCHIVE_MEDIA_TYPE` before streaming.

## 4. Size guard, undeliverable rows and honest reports (R2-04)

- [x] 4.1 RED: `crates/chatgpt-archive/tests/contract_projection.rs` `a_fact_over_the_event_limit_is_not_enqueued_and_the_import_is_partial` fails because the oversized fact is enqueued (three facts instead of two); `tests/normalized_events.rs` `an_undeliverable_row_does_not_wedge_the_pump` fails because the pass returns a `Broker(MaxPayloadExceeded)` error; `tests/config.rs` `max_event_bytes_defaults_and_is_bounded` fails because the key is not recognized. `transient_refusals_never_make_a_row_undeliverable` and `facts_under_the_event_limit_report_no_too_large_warning` pin negatives that already held, so they passed in the RED run. The four unit tests in `receipt/report.rs` were written beside a signature-only field stub and were not run red on their own; the integration test above covers the same behaviour end to end and was seen red.
- [x] 4.2 GREEN: `RATATOSKR__RECEIPT__MAX_EVENT_BYTES`, the guard in `build_import_facts` and the report assembly, `outbox_events.undeliverable_at`, the `max_payload` comparison in `publish_row`, skipping undeliverable rows, and the counter.

## 5. Blob lock protocol (R2-07)

- [x] 5.1 RED: `crates/chatgpt-archive/tests/privacy_deletion_closure.rs` `a_concurrent_import_of_identical_bytes_never_leaves_a_dangling_reference` (interleaving a) and `an_import_restores_an_artifact_blob_a_concurrent_eraser_removes` fail with a committed row that references a missing blob; `a_registration_that_cannot_restore_the_blob_fails_and_registers_nothing` (interleaving c) fails because the registration succeeds instead of failing with `BlobUnavailable`. `a_registration_that_commits_first_keeps_the_blob_for_the_eraser` (interleaving b) pins behaviour that already held, so it passed in the RED run. The erase gate seam, the `PublishRequest.staged_path` field and the `BlobUnavailable` variant were added first as signature-only scaffolding.
- [x] 5.2 GREEN: `blob_lock`, the eraser holding it for re-check and erase, every registrar taking it before the first referencing row and verifying presence, and the protocol replacing the residual-risk comment.

## 6. Operator example and variable names (R2-10)

- [ ] 6.1 RED: `services/chatgpt-archive/tests/boot.rs` `the_shipped_example_loads` fails because `deploy/systemd/chatgpt.conf.example` does not exist.
- [ ] 6.2 GREEN: the example, and `README.md` and `DEVELOPMENT.md` list the bus, receipt and size-guard variables.

## 7. Gate

- [ ] 7.1 Run `cargo fmt --all -- --check`, clippy with `-D warnings`, the 850-line check, `cargo test --workspace --locked`, `cargo deny check`, `openspec validate --all --strict` and `openspec validate --archived`; every RED listed above was run and observed failing for its stated reason before its GREEN.
