# Bootstrap the Ratatoskr ChatGPT archive service

## Why

The repository holds intent documents only; plan item 1 of `docs/IMPLEMENTATION_PLAN.md` requires the first runnable slice so that later import work lands on a working service instead of on prose. This change creates that slice: a Rust workspace whose binary boots with typed configuration, structured telemetry, health endpoints, typed errors, a content-addressed BlobStore adapter, and a first-version `chatgpt_archive` database schema.

## What Changes

- Add a cargo workspace (`crates/chatgpt-archive`, `services/chatgpt-archive`) with fleet-standard manifests, workspace lints, `clippy.toml` size limits, `deny.toml`, `rustfmt.toml`, and a pinned `rust-toolchain.toml`.
- Add typed, environment-loaded configuration (`RATATOSKR__SECTION__KEY`) with deny-unknown-fields parsing, redacted secrets, collected violations, and exit code 78 (EX_CONFIG) on failure.
- Add structured telemetry initialization: tracing with EnvFilter and JSON output on stdout, plus an explicit shutdown guard.
- Add an admin plane serving `/health/live`, `/health/ready`, `/metrics`, and `/version`, where readiness performs a real database round trip when a database is configured.
- Add typed errors (`thiserror`) with a closed client-visible failure taxonomy and a single HTTP error-envelope construction site; panics render static text and never leak internals.
- Add a BlobStore adapter following the stored-bytes contract cited from `ratatoskr-workspace` (`blob-references`): streaming SHA-256 while receiving, staging plus atomic publish, content-addressed layout `{root}/sha256/{xx}/{rest}`, owner `ratatoskr-chatgpt`, deterministic `BlobRef`s, verify-on-read. A backend seam keeps an S3-compatible implementation possible later; only the local filesystem backend ships now.
- Add `schema.sql` at the repository root defining the first version of the owned `chatgpt_archive` schema, applied by the persistence module inside one advisory-locked transaction. No migration tooling; later changes edit the file in place.
- Add a product CI gate (`.github/workflows/ci.yml`) with a PostgreSQL container, running fetch/deny/fmt/clippy/build/test plus the unchanged docs-only OpenSpec checks, and update `DEVELOPMENT.md` and `README.md` so documented commands match what CI runs.
- Out of scope: archive receipt, hashing pipelines beyond BlobStore, import runs, parsers, events, portable export (plan items 2+).

## Capabilities

### New Capabilities

- `runtime-configuration`: Typed environment-driven configuration with fail-closed validation and secret redaction.
- `health-and-telemetry`: Admin-plane liveness/readiness/metrics/version endpoints and structured logging setup.
- `error-reporting`: Client-safe failure classification and the single error-envelope rendering boundary.
- `blob-storage`: Content-addressed immutable byte storage addressed by `BlobRef` under owner `ratatoskr-chatgpt`.
- `archive-schema`: First-version `chatgpt_archive` PostgreSQL schema definition and its idempotent application.

### Modified Capabilities

(none — `openspec/specs/` starts empty)

## Impact

- New code: workspace manifests, two crates, `schema.sql`, `.github/workflows/ci.yml`; updates to `DEVELOPMENT.md` and `README.md` status.
- Dependencies: axum, tokio, sqlx (PostgreSQL, no migrate feature), figment, tracing/tracing-subscriber, thiserror, serde, secrecy, sha2, hex, uuid; `ratatoskr-identifiers` from `ratatoskr-contracts` as a rev-pinned git dependency for the shared `BlobRef` type.
- Systems: requires PostgreSQL 17 locally (compose.yaml provided); CI gains a Postgres service container.
- Fleet gates affected: first product manifest arrives together with `clippy.toml` (required by `fleet.yml`) and a product `ci.yml` that invokes `cargo test`.
