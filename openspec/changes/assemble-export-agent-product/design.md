## Context

Receipt currently publishes a terminal `partially_succeeded` report in the same transaction that
stores raw bytes. The existing state machine, parser registry, reconciliation model, immutable blob
store, PostgreSQL outbox, and admin readiness registry exist, but service startup does not compose
them into a supervised import path. Duplicate receipt returns before associating a new Platform
operation, and the outbox connects anonymously to one shared subject.

## Goals / Non-Goals

**Goals:** make receipt non-terminal, resume real imports after restart, report every correlated
operation from actual import truth, and require authenticated durable publication with truthful
readiness.

**Non-Goals:** provider login, a new API or schema version, migrations, storing credentials in the
database, changing the operation-report document, or deleting immutable raw evidence.

## Decisions

### Durable work and operation correlations are first-class current-schema rows

The current schema definition gains operation correlations and a bounded durable work projection.
Raw publication, correlation, and work enqueue happen in one transaction. A unique operation key
makes receipt replay idempotent; digest reuse points at the same export and import result without
discarding the new correlation. Development databases are recreated from the edited schema.

### One supervised worker owns post-receipt import

Service startup claims pending stored work with bounded leases, runs the existing inspection,
parser, reconciliation, and persistence path, and records the terminal counts/completeness before it
enqueues reports for every correlation. Claims are recoverable after process loss and terminal writes
are idempotent. Raw storage may expose progress internally but never a terminal Platform event.

### Publisher authentication and health are one runtime boundary

`ReceiptConfig` carries a redacted NKey seed-file path. The outbox connects with the fleet NKey
identity and publishes only to `evt.ai-archive.chatgpt.operation.reported.v1`. A stable runtime
health cell is failed before the first authenticated pass, failed on permission/connectivity errors,
and recovered by a successful acknowledged pass. The import worker has an equivalent liveness
projection. Archive readiness requires database, receipt store, worker, and publisher.

## Risks / Trade-offs

- A crash after broker acknowledgement can repeat an outbox message; stable message identity plus
  JetStream and Platform inbox idempotence make the replay safe.
- Parser failure must not poison the worker loop; each work item records its own bounded terminal
  failure while supervision continues.
- Exact normalized persistence may expose missing composition seams; those are implemented in this
  repository rather than replaced by fabricated counts.

## Migration Plan

Replace the development-only raw-terminal path in place, recreate test databases from `schema.sql`,
deploy the credentialed publisher and worker before Platform advertises ChatGPT archive readiness,
and retain raw archives/outbox rows on rollback.
