## Context

The repository is at architecture bootstrap: documents and OpenSpec scaffolding exist, no Rust manifest does. Fleet conventions were surveyed from sibling repositories; the structural template is `wt-threads-service-scaffold` (workspace shape, manifests, lint configuration) with richer runtime references in `ratatoskr-platform` (`crates/core` config/errors, `crates/http` admin plane and fault rendering, `crates/telemetry`, `crates/persistence`) and `ratatoskr-extractor` (`crates/blob-store`). The shared `BlobRef` type lives in `ratatoskr-contracts` `crates/identifiers`; cross-repository semantics are fixed by the store spec `blob-references`, cited from this change's blob-storage delta. Development status binds: first version only, no migrations, product name Ratatoskr.

## Goals / Non-Goals

**Goals:**

- One cargo workspace that builds green under the fleet gate (fmt, clippy `-D warnings`, deny check, tests) on the first commit.
- Runtime pieces small enough to be read in one sitting, each covered by a named test.
- BlobStore behavior identical in contract to the extractor reference so later import code needs no relearning.
- A schema definition that later plan items extend by editing one file.

**Non-Goals:**

- Archive receipt, ZIP handling, parsers, import runs, events, portable export.
- An actual S3 client dependency or credentials handling — only the seam.
- Docker packaging, release artifacts, NATS integration.
- Any second version of anything, migration tooling included.

## Decisions

### Workspace layout

Root manifest with `resolver = "3"`, members `crates/chatgpt-archive` (library, package `ratatoskr-chatgpt-archive`) and `services/chatgpt-archive` (binary `ratatoskr-chatgpt-archive`). Library modules: `config`, `telemetry`, `error`, `fault` (HTTP mapping), `admin` (health routes), `blob_store`, `persistence`. The binary is a thin `main` calling one `run()` lifecycle function. Alternative considered: single-crate bin — rejected; every fleet repository uses the lib-plus-service split and CI/test conventions assume it.

Dependency versions are pinned exactly (`=`) in `[workspace.dependencies]`, matching the scaffold's practice for a fresh tree; contracts come as rev-pinned git dependencies (`ratatoskr-identifiers`), the pattern platform uses.

### Configuration via a closed-key environment loader

Configuration loads through one module that reads `std::env::vars_os()`, keeps only `RATATOSKR__`-prefixed keys, and applies them against typed structs whose key set is closed in code; an unknown prefixed key is a violation, never ignored. Every entry is examined so one load reports all violations at once (`ConfigError { violations }`); secrets are `secrecy::SecretString` with skipped serialization and redacting `Debug`; failure prints a value-free report naming keys and rules and exits 78. The pure core takes an entry iterator (`from_environment`), so tests inject entries instead of mutating process state. Alternative considered: figment with `Env::prefixed("RATATOSKR__").split("__")` as platform does — rejected because the fleet's own service scaffold hand-rolls this exact contract dependency-free with identical testability, and this repository's configuration surface is small and flat.

### Telemetry

`tracing-subscriber` registry + `EnvFilter` + JSON fmt layer to stdout (`with_current_span(true).with_span_list(false)`); OpenTelemetry export is deliberately deferred — adding the OTLP stack now would add four crates and an exporter endpoint no deployment has yet. A `TelemetryGuard` owns shutdown explicitly (never in `Drop`). Default filter `"info"`.

### Admin plane

axum 0.8 with `default-features = false, features = ["http1", "json", "tokio"]`. One admin router serving `/health/live`, `/health/ready`, `/metrics`, `/version`, all wrapped with a `Cache-Control: no-store` layer. Readiness holds registered checks as ordered `(name, Box<dyn AsyncFn>)` pairs; the database round trip registers when a database is configured. Metrics render via the `metrics-exporter-prometheus` recorder installed at telemetry init. Version serves compile-time `env!("CARGO_PKG_VERSION")` plus `GIT_REV` from build metadata. The public API plane does not exist yet — plan item 2 introduces inbound endpoints; `run()` leaves the seam where it will mount.

### Errors

One boundary enum `ArchiveError { Rejected(FailureKind), Internal { subsystem, source } }` derived with thiserror; `FailureKind` is a closed taxonomy mapped to static `PublicFault { status, code, message, retryable }` entries with codes namespaced `chatgpt.*`. HTTP rendering has exactly one envelope construction site in `fault.rs`; handlers reject through a `reject(kind)` helper; `CatchPanicLayer` renders static-text 500s. The wire envelope shape follows the fleet error contract used by platform (status, code, message, retryable). Alternatives: per-layer error enums leaking into responses — rejected; it multiplies leak surfaces.

### BlobStore adapter

Facade struct `BlobStore` over an internal `BlobBackend` seam (async trait object) with one implementation, `LocalFsBackend`, mirroring the extractor reference: streaming SHA-256 while receiving, staging file `{root}/staging/ratatoskr-chatgpt-{uuid}.part`, atomic publish into `{root}/sha256/{2-hex}/{62-hex}` (create-new semantics so an existing digest is never rewritten), then verify-on-read comparing owner, algorithm, digest, length, media type before returning a path. Owner string is fixed to `ratatoskr-chatgpt` (matches the fleet owner-pattern regex). Media types validated against `type/subtype` form. The seam exists so a future S3-compatible backend implements `put_stream`/`open_read` without touching facade logic or callers; no remote SDK lands now. Tests target the facade only (per the spec scenario), never path internals.

### Schema application

`schema.sql` at repo root, embedded with `include_str!` into the persistence module. `apply_schema` opens one transaction, takes `pg_advisory_xact_lock` with a fleet-style constant key, checks presence with `to_regclass`-style probes, executes the DDL idempotently (`CREATE SCHEMA IF NOT EXISTS`, `CREATE TABLE IF NOT EXISTS`), commits. Pool via `PgPoolOptions` with bounded connections, acquire/idle timeouts, `test_before_acquire`. No sqlx migrate feature anywhere. First-version tables cover the AGENTS.md conceptual inventory that plan items 1–5 need placeholders for: accounts, exports, import_runs, projects, conversations, messages, message_relations, content_parts, assets, revisions, raw_records, completeness_reports, tombstones, outbox_events, inbox_events — minimal realistic columns now (identity, ownership, timestamps, provenance references), extended in place by later changes.

### Test harness

Plain `cargo test --workspace --locked` (the fleet has standardized on this; no nextest). Unit/integration tests live beside their modules and in `crates/chatgpt-archive/tests/`. Database-dependent tests skip unless `CHATGPT_TEST_DATABASE_URL` is set locally; CI always sets it via a digest-pinned PostgreSQL 17 service container. One boot test spawns the built binary with a temporary database-free environment and asserts `/health/live` answers and stdout emits structured JSON lines — this is the acceptance "service runs locally with health endpoints". `compose.yaml` provides local PostgreSQL matching the CI container.

### Gate

`.github/workflows/ci.yml`: gate job only (artifact/docker jobs wait until there is something to package): checkout with persist-credentials false, rust-cache SHA-pinned, Postgres service container, then `cargo fetch --locked`, `cargo deny check`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the 850-line-per-file awk check, `cargo build --workspace --locked`, `cargo test --workspace --locked`, and the ci-vs-DEVELOPMENT.md drift guard. `DEVELOPMENT.md` gets the exact command list so the guard passes. All third-party actions pinned by full commit SHA (zizmor enforces it).

## Risks / Trade-offs

- [Rev-pinned git dependency on ratatoskr-contracts] → pin the same rev for all contract crates; `deny.toml` allows exactly that git source; Dependabot keeps it current.
- [Exact version pins rot faster than caret pins] → acceptable for bootstrap; Dependabot updates them monthly like the rest of the fleet.
- [Boot test spawning the binary can be flaky on slow machines] → generous startup timeout, poll the health socket instead of sleeping a fixed interval.
- [Schema written before real export shapes are known] → intentional: development status says no data must survive schema changes, so early columns are cheap; raw evidence remains the source of truth.
- [Readiness depends on database presence] → database configuration is optional at bootstrap; with none configured, readiness reports ready with an empty check list rather than failing boot.
