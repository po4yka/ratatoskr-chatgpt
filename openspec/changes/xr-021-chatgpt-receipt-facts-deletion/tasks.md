## 1. Contracts pin

- [x] 1.1 Configuration, so no failing test first: move the five `ratatoskr-*` git dependencies to the XR-021 contracts commit and refresh `Cargo.lock`; the existing suite stays green.

## 2. Operator port

- [x] 2.1 RED: `crates/chatgpt-archive/tests/config.rs` expects the default operator listener `127.0.0.1:9085`; it fails because the default is 9084.
- [x] 2.2 GREEN: change the default in `src/config.rs`, `README.md` and `docs/testing/OWNER_FIXTURE_DISCOVERY.md`.

## 3. Receipt route (S06 D1, D2)

- [x] 3.1 RED: `tests/receipt_http.rs` `capabilities_document_is_served_without_claims` fails with 404, and `receipt_route_rejects_a_non_post_method_with_an_error_envelope` fails because a PUT answers an empty 405 instead of `chatgpt.request.method_not_allowed`.
- [x] 3.2 GREEN: add `GET /v1/capabilities` and the per-route 405 envelope fallback.
- [x] 3.3 REFACTOR: replace the local header names and the receipt path with the `platform_receipt` constants. No behaviour change, no new test.

## 4. Incomplete-import warning (S06 D3)

- [x] 4.1 RED: `an_incomplete_import_reports_partial_success_with_the_contract_warning` fails because `warnings` is empty and `validate()` returns `PartialWithoutDiagnostic`.
- [x] 4.2 GREEN: `imported()` sets `warnings: vec![incomplete_import_warning()]` when the status is `PartiallySucceeded`.

## 5. Complete envelopes and one generic pump (S07)

- [x] 5.1 RED: `tests/normalized_events.rs` `tombstone_is_published_as_a_contract_envelope` (real JetStream), unit tests `subject_for_event_type` and `pending_rows_include_unpublished_tombstones_in_id_order` fail because the pump selects only operation reports and tombstone rows hold bare payloads. `an_event_type_outside_the_closed_list_is_a_hard_error` and `the_schema_rejects_an_event_type_outside_the_closed_list` fail the same way. `a_refused_row_backs_off_without_starving_the_rows_behind_it` pins the retry columns and was written after the implementation, so it was never seen failing.
- [x] 5.2 GREEN: `NormalizedArchiveEvent` builds a full envelope, `schema.sql` constrains `event_type`, and the pump publishes every unpublished row through the closed subject mapping.

## 6. Fact projection (S07)

- [x] 6.1 RED: `tests/contract_projection.rs` fails because an import enqueues no `ai_archive.*` fact: ordered `archive.imported`, `project.added`, `conversation.added`; payload round trips and `AiConversationAdded::validate`; owner is the Platform user; unmapped account emits nothing; identical reparse adds nothing and changed content adds exactly one `conversation.updated`. The unmapped-account test pins a negative that already held before the change (no fact was ever enqueued), so it passed in the RED run; the other three failed with an empty fact list.
- [x] 6.2 GREEN: persist projects, conversation detail, assets and gaps; add `contract_projection.rs` with `build_import_facts`; call it from the initial import and the reparse persist path; add `OwnerResolver`.

## 7. Privacy deletion (S07)

- [x] 7.1 RED: `tests/privacy_deletion_closure.rs` `every_foreign_key_into_deletion_roots_is_handled` fails by listing the unhandled references. It turns green with 7.3 and 7.5 and is committed with them.
- [x] 7.2 RED: `archive_scope_completes_with_platform_operation_import_and_reparse_run`, `tenant_scope_completes_with_reparse_run`, `conversation_scope_completes_with_platform_operation_import` fail with the `platform_operation_imports_import_run_id_fkey` store error after the blob is already erased.
- [x] 7.3 GREEN: inventory categories `platform_operation_import` and `reparse_run`, deleted before import runs and exports.
- [x] 7.4 RED: `retained_project_conversation_and_asset_survive_deletion_of_their_first_seen_export` fails with `projects_first_seen_export_fkey`.
- [x] 7.5 GREEN: repoint retained provenance to the earliest retained observing export, else NULL, before deleting exports.
- [x] 7.6 RED: `rows_are_removed_before_bytes_and_a_crash_between_resumes` fails because `FinalizationFault::AfterRowsCommitted` does not exist as a behaviour and blobs are erased first.
- [x] 7.7 GREEN: rows phase, bytes phase and complete phase with `evidence_ref` and `rows_removed_at`; blob sharing recomputed against remaining rows under the tenant lock.
- [x] 7.8 RED: `tombstone_outbox_row_is_a_complete_event_envelope_with_platform_owner`, `unmapped_account_emits_no_tombstone_and_reports_unbound_count`, `tenant_deletion_keeps_unpublished_tombstones_of_earlier_requests` fail on the bare payload, the archive-internal owner and the removed earlier rows.
- [x] 7.9 GREEN: shared `tombstone_envelope`, `PrivacyDeletionService::with_platform_users`, unbound counting, and the outbox removal set that excludes unpublished tombstones.

## 8. Documentation and gate

- [x] 8.1 Documentation, so no failing test first: update `README.md` and `DEVELOPMENT.md`.
- [x] 8.2 Run `cargo fmt --all -- --check`, clippy with `-D warnings`, the 850-line check, `cargo test --workspace --locked`, `cargo deny check`, `openspec validate --all --strict` and `openspec validate --archived`; every RED listed above was run and observed failing for its stated reason before its GREEN.
