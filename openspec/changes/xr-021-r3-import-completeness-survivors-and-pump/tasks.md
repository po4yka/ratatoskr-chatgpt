## 1. Contracts pin (R3-16)

- [x] 1.1 Dependency pin, so no failing test can precede it (the existing suite staying green is the gate): move the `ratatoskr-*` git dependencies to `ad16855c4e7f3d52cd118274faa3b8f3ab4da576` and refresh `Cargo.lock`.

## 2. Inbox prefix (R3-01)

- [ ] 2.1 RED: `crates/chatgpt-archive/tests/inbox_prefix.rs` `the_pump_completes_jetstream_requests_under_its_inbox_prefix` times out because the pump uses the default inbox and the broker allows only `_INBOX_chatgpt.>`.
- [ ] 2.2 GREEN: `INBOX_PREFIX` applied with `custom_inbox_prefix` in one connect-options function, `deploy/nats/identity.conf`, and a CI step that puts `nats-server` on `PATH`.

## 3. One connection, event-id message names (R3-07 rules 1 and 4)

- [ ] 3.1 RED: `tests/normalized_events.rs` `the_pump_opens_one_connection_for_many_passes`, `an_empty_pass_makes_no_network_call` and `the_message_id_is_the_event_id` fail; `the_pump_reconnects_after_the_connection_is_closed` pins behaviour that already held.
- [ ] 3.2 GREEN: a lazily created shared client, an early return for an empty pass, replacement of a closed client, and `event_id` as `Nats-Msg-Id`.

## 4. Unmapped user is 403, refusals are logged (R3-03 rule 3, R3-07 rules 2 and 3)

- [ ] 4.1 RED: `tests/receipt_http.rs` `an_unmapped_platform_user_is_forbidden_not_unauthenticated`, `capabilities_carry_no_store` and `every_refused_platform_receipt_is_logged_with_a_class` fail; `malformed_claims_stay_unauthenticated` and `an_authorization_header_stays_unauthenticated` pin behaviour that already held.
- [ ] 4.2 GREEN: `FailureKind::AccountUnmapped`, logging for every refusal of the Platform receipt, and the `cache-control` header on the capability response.

## 5. A duplicate of a failed import is reported, a failed import is logged (R3-04 rule 2, R3-07 rule 2)

- [ ] 5.1 RED: `tests/import_state.rs` `a_second_upload_of_bytes_that_failed_permanently_is_reported_failed` and `a_permanent_import_failure_is_logged_with_a_class` fail.
- [ ] 5.2 GREEN: the selector takes `failed` runs with a bound unreported operation and `mark_permanent_failure` logs its class.

## 6. An import can be complete (R3-05)

- [ ] 6.1 RED: `tests/synthetic_conversations_parser.rs`, `tests/archive_asset_evidence.rs` and `tests/import_state.rs` completeness tests fail for the three gaps and the refused `assets.json`.
- [ ] 6.2 GREEN: remove the three gaps, verify `assets.json` through the blob store on the registered parser path, and update `docs/DOMAIN.md` and `README.md`.

## 7. Deleting an archive keeps retained records announced (R3-06)

- [ ] 7.1 RED: `tests/privacy_deletion_survivors.rs` tests with three cumulative exports fail because no survivor is re-announced and the deleted export's heads stay in the outbox.
- [ ] 7.2 GREEN: rebuild and enqueue the survivors in the row-removal transaction and remove the deleted exports' `archive.imported` heads.

## 8. Provisioning and documentation (R3-11, R3-19)

- [ ] 8.1 RED: `tests/config.rs` `the_example_role_file_exists` fails because `deploy/postgres/role.sql` does not exist.
- [ ] 8.2 GREEN: `deploy/postgres/role.sql`, a README deployment paragraph, and the rewritten status paragraphs of `DEVELOPMENT.md` and `README.md`.

## 9. Gate

- [ ] 9.1 Run `cargo fmt --all -- --check`, clippy with `-D warnings`, the 850-line check, `cargo test --workspace --locked`, `cargo deny --locked check`, `openspec validate --all --strict` and `openspec validate --archived`; every RED listed above was run and observed failing for its stated reason before its GREEN.
