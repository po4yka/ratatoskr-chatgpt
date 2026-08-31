## Why

ChatGPT raw receipts are currently treated as a terminal partial result even though parsing and
import may not have run, and a duplicate digest can leave a newly bound Platform operation without a
terminal report. The report publisher also cannot prove authenticated delivery to the secured bus.

## What Changes

- Make raw receipt persistence non-terminal and atomically enqueue restart-safe import work.
- Run the existing parser/import/completeness pipeline from service startup before emitting one
  truthful terminal operation report.
- Associate duplicate bytes with every bound Platform operation while retaining one raw archive and
  one normalized import result.
- Require configured NKey publication, durable outbox identity, and readiness that follows the
  import worker and report publisher.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `archive-receipt`: Raw storage enqueues durable import work and remains non-terminal.
- `import-state`: Import jobs resume after restart and produce actual completeness truth.
- `platform-operation-report`: Every bound operation receives one idempotent terminal result,
  including duplicate-byte receipts.
- `runtime-configuration`: The service requires a credentialed report publisher and exposes worker
  and publisher health.

## Impact

This changes the current ChatGPT schema, receipt/import worker and service startup, operation outbox
publication/configuration, readiness/admin projections, and synthetic PostgreSQL/NATS tests. Raw
archives remain immutable; no provider login, migration, second version, or private payload is added.
