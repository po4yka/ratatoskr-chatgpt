# Tasks: bootstrap-service-scaffold

## 1. Workspace skeleton (configuration — no behavior yet)

- [x] 1.1 Create root `Cargo.toml` (workspace, resolver 3, exact-pinned `[workspace.dependencies]`, workspace lints, release profile), `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml` (fleet size limits), `deny.toml`; verify with `cargo fetch --locked`. Configuration and toolchain files: no failing test can express a manifest that does not exist yet.
- [x] 1.2 Create `crates/chatgpt-archive` (lib, empty modules stubbed) and `services/chatgpt-archive` ([[bin]] `ratatoskr-chatgpt-archive`) with `.workspace = true` manifests; verify `cargo build --workspace --locked` succeeds.
- [x] 1.3 Add `.github/workflows/ci.yml` (gate: postgres container, fetch/deny/fmt/clippy/file-length/build/test, drift guard) and update `DEVELOPMENT.md` with the identical command list; verify the drift-guard grep matches locally. CI definition is configuration plus documentation; its first real run happens on push.
- [x] 1.4 Add `compose.yaml` with a digest-pinned PostgreSQL 17 service for local runs. Configuration only.

## 2. Typed configuration

- [x] 2.1 Add `crates/chatgpt-archive/tests/config.rs` with `minimal_valid_environment_parses` asserting required vars produce a fully-defaulted typed config, and `unknown_prefixed_key_fails_with_violation` asserting an unknown `RATATOSKR__SERVICE__NMAE` yields a violation naming that key. Run `cargo test --test config` and confirm both fail because the config module does not exist.
- [x] 2.2 Implement `src/config.rs`: figment loader (`RATATOSKR__` prefix, `split("__")`), typed structs with `deny_unknown_fields` and per-field defaults, `SecretString` database URL, validation collecting every violation, value-free diagnostics; verify tests from 2.1 pass plus new assertions for collected violations (`two_bad_values_report_together`) and secret redaction in Debug output (`secret_is_redacted_in_debug`).

## 3. Telemetry

- [x] 3.1 Add unit test `startup_record_is_valid_json_with_a_level_field` in `crates/chatgpt-archive/src/telemetry.rs` asserting every rendered startup log line is JSON with a level field; confirmed failing against the stub (empty render). Adapted from `guard_shutdown_is_idempotent`: `TelemetryGuard::shutdown(self)` consumes the guard, so a second call is unrepresentable at compile time and no runtime idempotency assertion exists to write.
- [x] 3.2 Implement `src/telemetry.rs`: registry + EnvFilter (default `info`, overridable via config) + JSON layer to stdout, Prometheus recorder with build-info gauge, explicit consuming `TelemetryGuard`; unit test passes (1 passed).

## 4. Errors and fault rendering

- [x] 4.1 Add `crates/chatgpt-archive/tests/error_reporting.rs`: `rejected_kind_maps_to_envelope` asserting a classified kind renders status/code/retryable from its static mapping, `internal_error_leaks_no_source_detail` asserting source-chain text never appears in the rendered body, `panic_renders_static_500` asserting a panicking handler yields a static-text 500 while the router stays usable. Confirm all three fail (no error/fault modules).
- [x] 4.2 Implement `src/error.rs` (`ArchiveError`, closed `FailureKind`, static `PublicFault` table) and `src/fault.rs` (single envelope construction site, `reject()` helper, catch-panic layer); verify the three tests pass.

## 5. Admin plane (health endpoints)

- [x] 5.1 Add `crates/chatgpt-archive/tests/admin_routes.rs`: `live_answers_without_database` (200, `state == "live"` when no DB configured), `ready_lists_failing_check_by_name` (503 + check name when a registered check fails), `version_reports_build_identity` (body contains crate version and git rev), `admin_responses_carry_no_store` (header present on every admin route). Confirm they fail.
- [x] 5.2 Implement `src/admin.rs`: admin router with `/health/live`, `/health/ready`, `/metrics`, `/version` behind a `Cache-Control: no-store` layer, ordered readiness checks; wire metrics recorder into telemetry init; verify tests pass.

## 6. BlobStore adapter

- [x] 6.1 Add `crates/chatgpt-archive/tests/blob_store.rs`: `identical_bytes_store_to_equal_reference_and_single_object`, `digest_matches_independently_hashed_bytes`, `interrupted_stream_leaves_no_final_object`, `corrupted_object_resolves_as_missing`, `foreign_owner_reference_does_not_resolve`, `invalid_media_type_is_rejected`. All target the facade only. Confirm they fail.
- [x] 6.2 Implement `src/blob_store.rs`: `BlobStore` facade over internal backend seam, `LocalFsBackend` with streaming SHA-256, staging publish under `{root}/sha256/{xx}/{rest}` with create-new semantics, verify-on-read against owner/algorithm/digest/length/media type, owner fixed to `ratatoskr-chatgpt`, `ratatoskr-identifiers` git dependency for `BlobRef`; verify the six tests pass.

## 7. Schema and persistence

- [x] 7.1 Write root `schema.sql`: first-version `chatgpt_archive` tables per design (accounts through inbox_events). Definition file; correctness is proven by 7.3's integration test, not by a unit test of SQL text.
- [x] 7.2 Implement `src/persistence.rs`: pool options from config, `apply_schema` embedding `include_str!("../../../schema.sql")` inside one advisory-locked transaction with idempotent DDL. No failing standalone test: pool construction has no observable behavior without a server, and 7.3 covers application end to end.
- [x] 7.3 Add `crates/chatgpt-archive/tests/persistence_schema.rs`, skipped unless `CHATGPT_TEST_DATABASE_URL` is set: `applying_creates_declared_relations` (every declared table exists after one apply) and `second_application_changes_nothing` (re-apply succeeds; relation set unchanged). Run once against compose Postgres and confirm green; CI exercises it with its own container.
- [x] 7.4 Register the database readiness check in the service wiring when `RATATOSKR__DATABASE__URL` is set, reusing the round trip as the check body; verify `ready_lists_failing_check_by_name` still passes with a bogus URL.

## 8. Binary lifecycle and boot acceptance

- [x] 8.1 Add `services/chatgpt-archive/tests/boot.rs`: `boot_serves_health_and_logs_structured_lines` spawning the built binary with a temp env, polling `/health/live` until 200, asserting stdout contains JSON log lines with a level field and `/health/ready` returns 200 with no database configured. Confirm it fails against the current stub binary.
- [x] 8.2 Implement `main.rs` + `run()`: load config (exit 78 on failure) → init telemetry → bind admin listener → build routes → mark ready → serve until SIGTERM/SIGINT → guard shutdown → exit 0. Verify the boot test passes.

## 9. Documentation truthfulness

- [x] 9.1 Update `README.md` status block: architecture bootstrap narrows to "scaffold implemented: service binary, health endpoints, BlobStore, schema"; importers/parsers remain planned. Documentation only.
- [x] 9.2 Update `DEVELOPMENT.md` "Current validation" to the product command list matching ci.yml exactly (drift guard depends on it). Documentation only.

## 10. Gate and archive

- [x] 10.1 Run the full gate locally: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, file-length awk check, `cargo deny check`, `cargo build --workspace --locked`, `cargo test --workspace --locked` (with `CHATGPT_TEST_DATABASE_URL` set), `git diff --check`, `openspec validate --all --strict`.
- [x] 10.2 Tick every task in this file, then archive the change with `openspec archive bootstrap-service-scaffold` and run `openspec validate --archived`.
