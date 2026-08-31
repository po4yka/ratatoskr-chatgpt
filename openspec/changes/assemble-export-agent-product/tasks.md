## 1. Restart-safe import

- [x] 1.1 RED — add `raw_receipt_is_nonterminal_until_restart_safe_import_completes`; observe raw
  persistence, restart, and assert the first terminal report follows actual import truth.
- [x] 1.2 GREEN — atomically enqueue receipt work, supervise the existing parser/import pipeline from
  startup, and make 1.1 pass.
- [x] 1.3 RED — add `duplicate_digest_reports_terminal_result_for_each_bound_operation`; assert one
  raw archive/import and one truthful result for each distinct Platform operation.
- [x] 1.4 GREEN — persist operation correlations for both new and duplicate receipts and enqueue
  idempotent operation-specific reports after import; make 1.3 pass.

## 2. Authenticated report runtime

- [x] 2.1 RED — extend outbox and readiness tests so missing/denied NATS credentials leave rows
  pending and keep the archive runtime not ready until authenticated publication recovers.
- [x] 2.2 GREEN — load a redacted NKey seed file, publish on the ChatGPT ingress subject, retain
  stable outbox identity, supervise worker/publisher health, and make 2.1 pass.

## 3. Verification

- [x] 3.1 Run focused RED/GREEN tests and the affected crate tests through `build-gate` with
  `--locked`.
- [x] 3.2 Run the repository's documented format, lint, build, test, schema, and OpenSpec gates; record
  any external broker/PostgreSQL requirement rather than marking an unrun check complete.
