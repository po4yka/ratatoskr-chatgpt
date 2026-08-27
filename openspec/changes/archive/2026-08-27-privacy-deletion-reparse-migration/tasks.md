## 1. Cross-repository deletion authority

- [x] 1.1 RED — add `user_requested_deletion_event_round_trips` to `crates/chatgpt-archive/tests/normalized_events.rs`; run the exact test through `build-gate` and confirm its assertion fails because the current contracts pin rejects `reason = "user_requested"`
- [x] 1.2 GREEN — after the contracts and Knowledge `AIARCH-009` commits are published, advance the contracts revisions in `Cargo.toml`/`Cargo.lock`, rerun the exact test, and verify the existing tombstone event name and payload shape are unchanged

## 2. Current schema and persisted provenance

- [x] 2.1 RED — add `schema_exposes_privacy_reparse_and_provenance_relations` to `crates/chatgpt-archive/tests/persistence_schema.rs`; apply the current schema twice against PostgreSQL 17 and confirm the relation/constraint assertions fail for missing observations, extracted artifacts, deletion, reparse, migration, and outbox-deduplication structures
- [x] 2.2 GREEN — edit only `schema.sql` in place to add the required current-schema relations/checks/uniqueness and make the exact schema test pass without a migration file, migration dependency, later schema version, or cross-schema foreign key

## 3. Exact BlobStore erasure

- [x] 3.1 RED — add `erase_is_exact_and_idempotent` to `crates/chatgpt-archive/tests/blob_store.rs`; run it through `build-gate` and confirm the assertion fails because an exclusively named object remains after erasure
- [x] 3.2 GREEN — add exact locally owned BlobStore erasure, preserve immutable store/verify behavior, and make two erasures of the same object succeed while sibling objects remain readable
- [x] 3.3 RED — add `erase_refuses_foreign_and_malformed_references` to `crates/chatgpt-archive/tests/blob_store.rs`; run it and confirm the refusal assertion fails because the erasure boundary does not yet validate owner/content-address resolution
- [x] 3.4 GREEN — enforce owner, digest algorithm, resolved-root, and exact-file checks and make the refusal test pass without touching any local object

## 4. Privacy deletion planning

- [x] 4.1 RED — add `deletion_inventory_enumerates_complete_scope` to new `crates/chatgpt-archive/tests/privacy_deletion.rs`; seed every required row/blob category, run it against PostgreSQL 17, and confirm the category/item equality assertion fails because the planner omits the closure
- [x] 4.2 GREEN — implement the tenant-locked deletion repository/planner and deterministic content-free inventory, including export observations and extracted artifacts, until every seeded category appears exactly once and totals are derived from items
- [x] 4.3 RED — add `deletion_scope_does_not_disclose_cross_tenant_subjects` to `privacy_deletion.rs`; run it and confirm the result/row-count assertion fails because cross-tenant and unknown scopes are not yet indistinguishable
- [x] 4.4 GREEN — add tenant authorization and not-found normalization at the planner boundary and make the cross-tenant test pass with no request or evidence mutation
- [x] 4.5 RED — add `conversation_plan_includes_containing_archives_and_only_unprovenanced_collateral` to `privacy_deletion.rs`; run it and confirm the retained/removed action assertion fails for a conversation observed in two exports with independently evidenced siblings
- [x] 4.6 GREEN — implement archive, conversation, and tenant closure rules from explicit observations and make the scope-semantics test pass without absence-based deletion

## 5. Privacy deletion execution

- [x] 5.1 RED — add `deletion_finalization_is_atomic_with_audit_and_tombstones` to `privacy_deletion.rs` with a hand-written persistence fault at finalization; run it and confirm the assertion observes a partial database effect instead of all removal/audit/outbox effects rolling back together
- [x] 5.2 GREEN — implement plan/purge/finalize execution, stable completion evidence, one owned transaction for database removal/audit/deduplicated `user_requested` tombstones, and resumable state until the atomicity test passes
- [x] 5.3 RED — add `completed_deletion_replay_returns_original_report` to `privacy_deletion.rs`; run it and confirm replay duplicates an audit, tombstone, or removal count
- [x] 5.4 GREEN — reconcile request state by stable id and uniqueness constraints so replay returns the original report with no new effect
- [x] 5.5 RED — add `tenant_deletion_retains_blob_referenced_by_another_tenant` to `privacy_deletion.rs`; run it and confirm byte-identical shared content becomes unreadable to the survivor
- [x] 5.6 GREEN — add the fresh reachability proof under the tenant privacy gate and make shared items report retained-shared while exclusive raw/extracted blobs verify absent

## 6. Privacy deletion command boundary

- [x] 6.1 RED — add `privacy_delete_plan_requires_exactly_one_tenant_scope` and `privacy_delete_execute_requires_confirmation` to new `services/chatgpt-archive/tests/privacy_delete_command.rs`, running each separately and confirming the invalid-invocation assertions fail with the current parser
- [x] 6.2 GREEN — implement `privacy-delete plan` and `privacy-delete execute` grammar, exclusive scope validation, request identity, `--confirm`, stable JSON stdout, diagnostic stderr, and exit 0/1/2 mapping until both process-boundary tests pass

## 7. Parser registry version discovery

- [x] 7.1 RED — add `compatible_versions_and_exact_lookup_are_deterministic` to `crates/chatgpt-archive/tests/parser_registry.rs`; run it and confirm ordering/exact-resolution assertions fail while ordinary ambiguity remains unchanged
- [x] 7.2 GREEN — add comparable declared parser versions, exact compatible lookup, deterministic discovery, and the compiled parser executor boundary without making ordinary intake select through ambiguity

## 8. Reparse planning and apply

- [x] 8.1 RED — add `reparse_dry_run_matches_immediate_apply_without_writes` to new `crates/chatgpt-archive/tests/reparse.rs` with hand-written deterministic v1/v2 parsers; run it and confirm the dry/apply report equality or zero-write assertion fails
- [x] 8.2 GREEN — implement verified raw read, hostile reinspection/extraction, exact newer parser execution, validation/reconciliation, immutable fingerprinted plan, JSON report, and apply transaction through one comparison path until dry-run fidelity and side-effect freedom pass
- [x] 8.3 RED — add `reparse_apply_is_idempotent_for_same_fingerprints` to `reparse.rs`; run it and confirm the second apply creates a duplicate run, revision, extracted artifact, or outbox event
- [x] 8.4 GREEN — add applied-run uniqueness and prior-result reconciliation so the second apply returns the original report with zero new evidence
- [x] 8.5 Verification — add `reparse_omission_retains_existing_subject_with_warning` to `reparse.rs`; the shared comparison implemented for 8.2 already made this pass, so no false RED is claimed
- [x] 8.6 GREEN — classify omissions as coverage warnings/proposed removals, preserve the projection, and emit no deletion event
- [x] 8.7 RED — add `reparse_command_requires_tenant_archive_parser_and_preserves_dry_run` to new `services/chatgpt-archive/tests/reparse_command.rs`; run it and confirm required-argument/JSON/exit assertions fail
- [x] 8.8 GREEN — implement the `reparse` operator command with exact `NAME@VERSION`, optional `--dry-run`, stable JSON stdout, redacted diagnostics, and the documented exit mapping

## 9. Parser-version migration reports

- [x] 9.1 RED — add `migration_report_classifies_each_archive_once_and_derives_totals` to new `crates/chatgpt-archive/tests/parser_migration.rs`; run it with reordered eligible/current/unsupported/missing/privacy-blocked inputs and confirm entry order or summary totals are wrong
- [x] 9.2 GREEN — implement deterministic tenant-scoped migration planning over the reparse planner and derive every total from sorted per-archive entries until serialized reports are identical across input order
- [x] 9.3 Verification — add `migration_apply_reports_partial_when_one_archive_fails` to `parser_migration.rs`; independent apply was already present when the integration test was introduced, so no false RED is claimed
- [x] 9.4 GREEN — apply eligible entries independently through reparse, retain non-eligible plan outcomes, and return an explicit partial report while preserving successful archives
- [x] 9.5 RED — add `parser_migrate_command_requires_tenant_parser_and_preserves_dry_run` to new `services/chatgpt-archive/tests/parser_migrate_command.rs`; run it and confirm invocation/report/exit assertions fail
- [x] 9.6 GREEN — implement the `parser-migrate` command with stable JSON, dry-run side-effect freedom, and exit 1 for operational partial results without adding database migration artifacts or tooling

## 10. Owner-provided fixture discovery and golden admission

- [x] 10.1 RED — add `fixture_admission_rejects_raw_private_or_unapproved_candidates` to new `crates/chatgpt-archive/tests/fixture_admission.rs`; run each table case separately and confirm at least one raw archive, forbidden field, unsafe path, missing review, or nondeterministic expectation is incorrectly admitted
- [x] 10.2 GREEN — implement strict manifest parsing, structural comparison, privacy/secret/path checks, review/approval gates, deterministic report ordering, and the `fixture-admit --candidate PATH` command until every unsafe candidate is refused and one fully synthetic candidate is admitted
- [x] 10.3 Documentation cannot start from a failing behavior test because it records the owner-only operational process; add `docs/testing/OWNER_FIXTURE_DISCOVERY.md` and README links covering consent, private storage, production inspection, minimization/redaction, comparison, review, admission, golden blessing, support claims, and disposition, then verify every command/path exists and no real export or personal value is tracked
- [x] 10.4 Add one non-sensitive derived-fixture manifest and read-only golden contract test, run it through `build-gate`, inspect every golden line, and verify normal tests never bless or rewrite fixtures

## 11. Refactor, full gate, and delivery

- [x] 11.1 Refactor only after all targeted tests are green: review public APIs with the Rust checklist, split oversized modules within repository limits, keep transactions/locks in stable order, and rerun affected crate tests with no behavior change
- [x] 11.2 Run `cargo fmt --all -- --check` and the full fenced gate from `DEVELOPMENT.md`, wrapping every compiler-backed Cargo top-level command in `build-gate`, with `CHATGPT_TEST_DATABASE_URL` targeting PostgreSQL 17; also run `git diff --check` and `openspec validate --all --strict`
- [x] 11.3 Review the final diff for private data, raw fixtures, migration files/tooling, later majors, fake support claims, stale generated output, unbounded telemetry labels, unrelated changes, and missing call sites; verify `git status` contains only this change
- [x] 11.4 Sync all seven delta specs, verify no delta remains, archive this change, run `openspec validate --archived`, then commit, integrate the task branch into `main`, push `main`, and only after remote verification remove the dedicated worktree and merged branch
