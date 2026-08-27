## 1. Portable state and deterministic package

- [x] 1.1 Add `crates/chatgpt-archive/tests/portable_export.rs::identical_state_produces_byte_identical_zip`, which builds one tenant-scoped fixture state twice and asserts unequal pre-feature output is replaced by equal ZIP bytes and a fixed SHA-256 golden digest; run it and confirm the output-export assertion fails rather than compilation.
- [x] 1.2 Implement the portable state model, canonical JSON/Markdown renderer, fixed-metadata ZIP writer, and atomic output publication in `portable_export`; verify `identical_state_produces_byte_identical_zip` passes.

## 2. Provenance, assets, and filtering

- [x] 2.1 Add `manifest_lists_json_markdown_and_verified_asset_members` in `crates/chatgpt-archive/tests/portable_export.rs`, asserting the initial exporter lacks all member digests/provenance and cannot copy a verified asset; run it and confirm that assertion fails.
- [x] 2.2 Implement manifest generation, provenance headers, verified `BlobStore` asset copying, unavailable-asset warnings, and failure cleanup; verify `manifest_lists_json_markdown_and_verified_asset_members` and `unreadable_verified_asset_aborts_without_archive` pass.
- [x] 2.3 Add `filters_limit_export_to_matching_project_and_observed_time` and `tenant_scope_excludes_other_account_evidence` in `crates/chatgpt-archive/tests/portable_export.rs`; assert an unimplemented filter/source includes non-matching records; run each and confirm the selection assertion fails.
- [x] 2.4 Implement deterministic tenant, project, and inclusive observed-time selection and report filter values in the manifest; verify both filtering tests pass.

## 3. Persistent command surface

- [x] 3.1 Add `services/chatgpt-archive/tests/portable_export_command.rs::portable_export_command_requires_tenant_and_output` and a repository contract test proving tenant-filtered loading; run them and confirm missing command/repository behavior fails at an assertion.
- [x] 3.2 Implement the tenant-scoped PostgreSQL portable-export read model over the owned first-version schema and the service `portable-export` command; verify the command/repository tests pass without a migration file.

## 4. Validation and documentation

- [x] 4.1 Run the focused archive and service test suites, format, clippy, workspace build/tests, `git diff --check`, `openspec validate --all --strict`, and `openspec validate --archived`; inspect the final diff and record observed outcomes.
