# Developing Ratatoskr ChatGPT Archive

> Status: Proposed  
> Last reviewed: 2026-08-20

The importer, parser registry, schema, storage, privacy deletion, reparse and portable exporter are implemented and tested against synthetic exports; the Compliance adapter and a parser for a real ChatGPT export are planned and not implemented.

## Intended toolchain

Rust/Tokio, safe ZIP/archive handling, streaming SHA-256, SQLx/PostgreSQL, content-addressed BlobStore, Serde/JSON Schema, NATS, fixture-driven parsers, tracing, and testcontainers.

## Code size limits

`clippy.toml` carries the function and signature limits, and the 850-line cap on a Rust file is checked by the `gate` job of `.github/workflows/ci.yml`. `fleet.yml` fails the gate when a manifest arrives without a lint configuration.

`ratatoskr-workspace/docs/QUALITY_GATES.md` holds the numbers the repositories with code use today, the command that measured each one, and the limits that were rejected with the reason. Read it before you choose numbers, then measure this tree. Each limit is set at the worst case the tree already has, so that the check fails on a regression and not on work that has not been done yet.

## Current validation

The docs-only OpenSpec checks and the Rust product gate both run. The full local
equivalent of CI is:

```bash
git diff --check
openspec validate --all --strict
openspec validate --archived
```

`.github/workflows/openspec.yml` runs the two OpenSpec commands in CI. The product gate lives in
`.github/workflows/ci.yml` beside them; it needs a PostgreSQL for the schema integration tests.

### Rust — also the CI gate

```bash
cargo fetch --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --workspace --locked
cargo test --workspace --locked
```

`cargo deny check` is not in this list: it runs in its own `deny` job in `ci.yml`, separate from
the gate, so a new RustSec advisory cannot hide a clippy or test failure behind it. It reads
RustSec advisories, the licence set, duplicate versions and the source policy. Run it locally with
`cargo deny check` before a commit the same as the gate list above.

This list and the `- run: cargo` lines in `ci.yml`'s `gate` job are enforced identical by that
workflow's last step. `CHATGPT_TEST_DATABASE_URL` must point at a PostgreSQL 17 for
`crates/chatgpt-archive/tests/persistence_schema.rs`; without it those tests skip locally, while CI
always sets it from its service container.

The archive-fact, privacy-deletion and event-pump tests are stricter: each creates its own disposable
database on that server, so the URL's role needs `CREATEDB`, and the pump tests publish to a real
JetStream named by `CHATGPT_TEST_NATS_URL` (a `nats-server -js`). Those tests fail rather than skip
when either variable is unset, because a skipped broker test proves nothing about delivery. CI starts
the broker in the `Start disposable JetStream` step of `ci.yml`; locally `docker compose up -d postgres nats` starts both, on `127.0.0.1:5439` and `127.0.0.1:4229`.

More test files start a private `nats-server` of their own, so the binary must be on `PATH`: `inbox_prefix.rs` and `deletion_ack_consumer.rs` run the pump and the acknowledgement consumer against a broker whose only service user is the stanza of `deploy/nats/identity.conf`, `tombstone_resend.rs` proves the resend against a plain broker whose events stream has a one second duplicate window, `stream_sequence.rs` and `stream_erasure_request.rs` run the pump against plain brokers holding the events and commands streams, and the connection-count tests read the server's monitoring endpoint. CI copies the binary out of the pinned NATS container in the `Put nats-server on PATH` step; locally `brew install nats-server` or a copy from the container does the same. `CHATGPT_TEST_PORT_BASE` confines those private brokers to twenty loopback ports from that base; without it the operating system picks the ports.

## Deployed configuration

`deploy/systemd/chatgpt.conf.example` is the operator example; `services/chatgpt-archive/tests/boot.rs` (`the_shipped_example_loads`) loads it through `Config::from_environment`. The receipt listener (`RATATOSKR__RECEIPT__LISTEN_ADDRESS`, default `127.0.0.1:8096`), the operator listener (`RATATOSKR__ADMIN__LISTEN_ADDRESS`, default `127.0.0.1:9085`), the event bus (`RATATOSKR__RECEIPT__EVENT_BUS_URL` and `RATATOSKR__RECEIPT__EVENT_BUS_NKEY_SEED_PATH`), the account mapping (`RATATOSKR__RECEIPT__PLATFORM_ACCOUNTS`) the enqueue-time size guard (`RATATOSKR__RECEIPT__MAX_EVENT_BYTES`) and the tombstone resend interval and cap (`RATATOSKR__TOMBSTONE__RESEND_AFTER_SECONDS`, `RATATOSKR__TOMBSTONE__RESEND_CAP_SECONDS`) are described in `README.md` under "Deployed configuration". A change to any of these keys updates the example in the same commit.

## Stream erasure bookkeeping

Deleting this service's outbox rows does not delete what the pump already published, so every published copy of a deleted fact needs a `platform.stream_erasure.requested.v1` command that names its stream sequence (XR-021 CONTRACTS.md R5-12 and R6-03). The protocol that makes the record complete:

1. `outbox_events.published_stream_seqs bigint[]` holds the sequence of every PubAck the pump received for the row, ascending and without duplicates. A duplicate PubAck (the same `Nats-Msg-Id` inside the stream's 120 s duplicate window) carries the original sequence and adds nothing; a republish after the window is a new copy and adds a second sequence.
2. The first publication of a row is one short transaction per row: `select 1 ... for update skip locked`, publish, await the PubAck, `update ... set published_at, published_stream_seqs` and commit. The batch of candidates is chosen without a lock; the lock is taken per row, on one connection at a time. A row that is gone, already published or held by a deletion is skipped and nothing is published for it. The update must affect exactly one row: anything else returns `PublishError::MarkLost` and increments `chatgpt_archive_outbox_mark_lost_total`. Skipped rows are counted in `chatgpt_archive_outbox_skipped_total`.
3. The lock is held for at most the JetStream request timeout, which the pump sets explicitly to 5 s (`PUBLISH_TIMEOUT` in `receipt/outbox/mod.rs`). The pump never waits on a lock, so it cannot take part in a deadlock. A deletion that meets the lock waits for the pump, bounded by the same 5 s, and then, under READ COMMITTED, deletes the committed row version and reads its sequences from the `DELETE ... RETURNING` that removes it. The pool consequence is that the pump holds one connection for as long as a publish is in flight, so the pool must keep a connection free for the deletion. The resend of an already published tombstone and `mark_undeliverable` keep their own short statements and take no lock.
4. A PubAck whose commit failed is carried in memory (`receipt/outbox/carry.rs`, content-bearing facts only): the next successful mark of that row merges the carried sequences into the array. At the start of every pass the pump checks each carried entry against the table; an entry whose row is gone is a copy no deletion knew about, so the pump enqueues the erasure command itself through the same function a deletion uses (`stream_erasure.rs`). That command's `request_ref` and outbox `aggregate_id` are `privacy_request:<event id of the orphaned row>`, which names no real deletion request, and its deduplication key is `stream-erasure:reconcile:<event id>:<index>`, so a second pass enqueues nothing more.

The one residual that remains: a process crash (or a database that stays down until the process restarts) between the broker's PubAck and the commit of its sequence loses the in-memory carry. A restart inside the 120 s duplicate window is covered, because the republish is a duplicate PubAck with the original sequence. After the window the republish is a new copy and the first copy is named by no record; it ages out of the stream at 7 days or 1 GiB. Closing it needs Edge to find a message by its `Nats-Msg-Id`, a new field on the erasure command and so a contract addition, which is not done (R6-05).

The test seam is `PublishProbe` (`receipt/probe.rs`), modelled on `privacy_deletion::EraseGate`: production never sets one. It pauses the pump after the candidates are chosen or after the PubAck, or fails one commit. `tests/stream_erasure_in_flight.rs` drives it against a real broker.

## Workflow

1. Treat every export as hostile and persist the immutable raw archive before parsing.
2. Detect format/schema and choose an explicit versioned parser.
3. Preserve unknown records/content parts and produce a completeness report.
4. Reconcile projects, conversation graphs, revisions, files, Canvas/assets without overwriting history.
5. Test limits, traversal, duplicates, partial assets, interruption, privacy deletion, and portable export.

The first scaffold PR must document exact commands. Default tests use synthetic export fixtures and never ChatGPT login/session cookies or personal exports.

## What a clone needs before you plan a change

A change is planned with OpenSpec, which is a CLI a clone installs for itself. Use the version
`.github/workflows/openspec.yml` pins, so your terminal and the gate answer the same:

```bash
npm install --global @fission-ai/openspec@1.10.0
```

Cross-repository behaviour lives in a store, and registering one is per-machine state that no
repository can turn on for you — the same kind of step as `git config core.hooksPath .githooks`:

```bash
git clone git@github.com:po4yka/ratatoskr-workspace.git <path>
openspec store register <path> --id ratatoskr-workspace
```

`openspec doctor` reports whether both are in place.

## The Rust skills in this repository

`.agents/skills/` holds eighteen Rust skills vendored from `po4yka/rust-skills`, and
`.claude/skills/` symlinks to them. Unlike the steps above this needs nothing from your machine: the
files are in the tree, so a fresh clone already has them.

Update them with the catalogue and never by hand:

```bash
npx skills update
```

That rewrites `.agents/skills/` and `skills-lock.json` from the catalogue. Run it in one repository,
read the diff, then apply the same change to every Ratatoskr repository whose stack is Rust.
`ratatoskr-workspace/.github/workflows/drift.yml` fails when one copy differs from the others.
