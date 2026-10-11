## 1. Contracts pin (R3-16)

- [x] 1.1 Dependency pin, so no failing test can precede it (the existing suite staying green is the gate): move the `ratatoskr-*` git dependencies to `ad16855c4e7f3d52cd118274faa3b8f3ab4da576` and refresh `Cargo.lock`.

## 2. Inbox prefix (R3-01)

- [x] 2.1 RED: `crates/chatgpt-archive/tests/inbox_prefix.rs` `the_pump_completes_jetstream_requests_under_its_inbox_prefix` times out because the pump uses the default inbox and the broker allows only `_INBOX_chatgpt.>`.
- [x] 2.2 GREEN: `INBOX_PREFIX` applied with `custom_inbox_prefix` in one connect-options function, `deploy/nats/identity.conf`, and a CI step that puts `nats-server` on `PATH`.

## 3. One connection, event-id message names (R3-07 rules 1 and 4)

- [x] 3.1 RED: `tests/normalized_events.rs` `the_pump_opens_one_connection_for_many_passes` (20 connections), `an_empty_pass_makes_no_network_call` (connection refused) and `the_message_id_is_the_event_id` (header was the row number) failed, as did `pending_rows_include_unpublished_tombstones_in_id_order` once it compared the header with the event id; `the_pump_reconnects_after_the_connection_is_closed` (a broker restart) pins behaviour that already held. `a_closed_connection_is_replaced_by_the_next_pass` needs the new `close()` method and was written with the GREEN step, so it was not run red.
- [x] 3.2 GREEN: a lazily created shared client, an early return for an empty pass, replacement of a closed client, and `event_id` as `Nats-Msg-Id`.

## 4. Unmapped user is 403, refusals are logged (R3-03 rule 3, R3-07 rules 2 and 3)

- [x] 4.1 RED: `tests/receipt_http.rs` `an_unmapped_platform_user_is_forbidden_not_unauthenticated`, `capabilities_carry_no_store` and `every_refused_platform_receipt_is_logged_with_a_class` fail; `malformed_claims_stay_unauthenticated` and `an_authorization_header_stays_unauthenticated` pin behaviour that already held. A scoped tracing subscriber raced with the other tests of the binary for the callsite cache, so the log test uses one global subscriber that routes events by emitting thread.
- [x] 4.2 GREEN: `FailureKind::AccountUnmapped`, logging for every refusal of the Platform receipt, and the `cache-control` header on the capability response.

## 5. A duplicate of a failed import is reported, a failed import is logged (R3-04 rule 2, R3-07 rule 2)

- [x] 5.1 RED: `tests/import_state.rs` `a_second_upload_of_bytes_that_failed_permanently_is_reported_failed` and `a_permanent_import_failure_is_logged_with_a_class` fail.
- [x] 5.2 GREEN: the selector takes `failed` runs with a bound unreported operation and `mark_permanent_failure` logs its class.

## 6. An import can be complete (R3-05)

- [x] 6.1 RED: after a signature-only change (the executor returns a boxed future and its input carries the raw archive and the blob store, behaviour unchanged), `tests/import_state.rs` `an_export_with_only_conversations_is_complete`, `an_export_with_projects_canvas_and_verified_assets_is_complete` and `a_declared_asset_with_no_file_is_a_missing_file_gap` failed on the three `*_not_observed` gaps (and the refused `assets.json`), `an_unrecognized_record_is_still_a_gap` failed on the extra gaps, and `tests/archive_asset_evidence.rs` `an_assets_json_with_every_file_present_imports_the_assets` failed with the refusal; `a_malformed_assets_json_fails_permanently` and `a_malformed_assets_json_is_a_parser_failure` pin behaviour that already held. Running them showed that every mapping node repeating its own key as `id` was recorded as an unrecognized record, so no conversation could be complete: `tests/synthetic_conversations_parser.rs` `a_mapping_node_id_equal_to_its_key_is_not_an_unrecognized_record` was added and failed for that reason (`a_mapping_node_id_that_differs_from_its_key_is_preserved` pins the evidence that stays).
- [x] 6.2 GREEN: remove the three gaps, run the registered parser through the same asset verification as `parse_archive` (the executor stores each entry by digest and verifies the declared files against the blob store), record a node `id` only when it differs from its mapping key, update `tests/receipt_repository_pg.rs` and `tests/contract_projection.rs` to the new truth (a complete export has no gap and no incomplete-import warning), and update `docs/DOMAIN.md` and `README.md`.

## 7. Deleting an archive keeps retained records announced (R3-06)

- [x] 7.1 RED: reading the code for the survivor repair showed that the importer never shares a conversation between exports (`conversations` has no unique key on the account and provider id, so each export created its own row with a new id and `ON CONFLICT DO NOTHING` conflicted with nothing), which makes every record belong to exactly one export and the hazard of R3-06 unreachable for conversations. `tests/cumulative_exports.rs` `a_conversation_in_two_exports_is_one_row_observed_by_both` (five rows instead of three) and `a_changed_conversation_keeps_its_identity_and_gains_a_revision` (two rows instead of one) failed.
- [x] 7.2 GREEN: `UNIQUE (account_id, external_id)` on `conversations`, the importer resolves the row with `ON CONFLICT (account_id, external_id) DO UPDATE ... RETURNING id`, decides `unchanged` from the latest revision of that shared row, and an unchanged conversation still moves `last_seen_export` and observes its messages for the export. The two seeds of `tests/parser_migration.rs` that created the same conversation twice for one account now share the row.
- [x] 7.3 RED: `tests/privacy_deletion_survivors.rs` `deleting_the_oldest_export_reannounces_its_survivors` (nothing followed the tombstones), `a_survivor_already_named_under_a_retained_archive_is_not_reannounced` (deleting E2 announced nothing for C) and `the_tombstone_and_the_repair_commit_together` (after the injected `AfterTombstones` fault and a re-execution nothing followed the tombstones) failed with three cumulative exports; `deleting_every_export_announces_no_survivor` and `the_deleted_exports_head_facts_leave_the_outbox` pin behaviour that already held (every outbox row bound to a deleted export leaves with it). The `FinalizationFault::AfterTombstones` seam was added first as a rollback after the tombstone insert.
- [x] 7.4 GREEN: before the scope's rows are removed the deletion finds the retained projects and conversations whose latest announced fact names a deleted archive; after the tombstones it enqueues an `updated` fact for each, built from the rows as they now stand, naming the earliest retained observing export (by `received_at`, then export id) and stamped a millisecond after the tombstone, in the same transaction.

## 8. Provisioning and documentation (R3-11, R3-19)

- [ ] 8.1 RED: `tests/config.rs` `the_example_role_file_exists` fails because `deploy/postgres/role.sql` does not exist.
- [ ] 8.2 GREEN: `deploy/postgres/role.sql`, a README deployment paragraph, and the rewritten status paragraphs of `DEVELOPMENT.md` and `README.md`.

## 9. Gate

- [ ] 9.1 Run `cargo fmt --all -- --check`, clippy with `-D warnings`, the 850-line check, `cargo test --workspace --locked`, `cargo deny --locked check`, `openspec validate --all --strict` and `openspec validate --archived`; every RED listed above was run and observed failing for its stated reason before its GREEN.
