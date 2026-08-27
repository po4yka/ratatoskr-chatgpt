## 1. Testable parser boundary and synthetic evidence

- [x] 1.1 Add the compiling public `SyntheticConversationsParser` and normalized record seam plus `tests/fixtures/synthetic_conversations.json`; verify the existing `cargo nextest run --locked -p ratatoskr-chatgpt-archive` suite remains green. This API/fixture prerequisite cannot start with a failing behavioral test because the public parser types and synthetic input must compile before an external test can call them.
- [x] 1.2 Add `crates/chatgpt-archive/tests/synthetic_conversations_parser.rs` with `synthetic_fixture_maps_conversations_messages_and_parts`, `successful_parse_carries_schema_and_parser_version`, `parsing_identical_fixture_is_deterministic`, and `unknown_fields_and_parts_remain_losslessly_available`; run the target and confirm every assertion fails because the seam returns its conservative unsupported result, not because a test fails to compile.
- [x] 1.3 Implement the synthetic grammar parser, registration declaration, typed text/tool/media-reference mapping, parser/schema stamps, deterministic ordering, and raw-field/unknown-part preservation; rerun `cargo nextest run --locked -p ratatoskr-chatgpt-archive --test synthetic_conversations_parser` and confirm all four tests pass.

## 2. Scope documentation and regression validation

- [x] 2.1 After the parser tests are green, update `README.md` and any affected parser documentation to identify the supported schema as synthetic-only and state that real-export validation requires an owner-authorized private fixture; verify `git diff --check` and the relevant documentation links/readmes.
- [x] 2.2 Run `git diff --check`, `openspec validate --all --strict`, `openspec validate --archived`, `cargo fetch --locked`, `cargo deny check`, `cargo fmt --all -- --check`, `build-gate -- cargo clippy --workspace --all-targets --locked -- -D warnings`, the tracked-Rust file-length check, `build-gate -- cargo build --workspace --locked`, and `build-gate -- cargo test --workspace --locked`; inspect the final diff and tick every completed task.

## 3. Delivery

- [x] 3.1 Archive `synthetic-conversations-parser` with OpenSpec and confirm `openspec validate --archived` passes; commit the dedicated worktree branch, fast-forward `main`, push `main`, and remove only this task worktree and branch after the push is observed. This delivery task has no failing unit test because it archives and publishes already-validated work.

## Follow-up blocker outside this change

- Owner-authorized real ChatGPT export fixture is required before a golden test
  can validate any real provider schema. It is absent in this session, must not
  be committed, and must drive a new production-parser OpenSpec change after
  safe reduction/redaction.
