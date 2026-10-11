## Context

XR-021 CONTRACTS.md is the single source of truth. This change implements the ChatGPT half of its Round 3 amendments and records only decisions that are local to this repository. Claude's equivalent of the survivor repair is commit `71e692c` in the Claude repository; it is the model, the tables here differ (`export_entity_observations`, `last_emitted_digest`).

## Decisions

- The inbox prefix is one `const INBOX_PREFIX` beside the connect function `connect_options`, which every connection of this service goes through. `deploy/nats/identity.conf` is a byte-equal copy of the `CHATGPT` stanza of Platform's `ratatoskr.conf`; the test starts a `nats-server` whose only user is that stanza.
- The client lives in a `tokio::sync::Mutex<Option<Client>>` shared by clones of the outbox. An empty pass returns before the lock is taken. A client whose `connection_state()` is `Shutdown`-like (async-nats reports a closed client as the `Disconnected` state after its connection task ended, and publishes then fail) is replaced after a failed pass; async-nats reconnects by itself while it is merely disconnected.
- `Nats-Msg-Id` is the envelope `event_id`, which is already inside the stored envelope. The row id stays the ordering key.
- `AccountUnmapped` is a new `FailureKind` (403, `chatgpt.account.unmapped`). `platform_claims` keeps returning `None` for malformed claims (401) and the handler distinguishes an unmapped user by looking the claimed user up after the claims parsed.
- Refusals are logged through the same `boundary.log()` call that `create_export` already uses, so the class vocabulary is one table.
- A `failed` run enters the selector only when a bound operation has `reported_at is null`; reporting reuses the code path of `mark_permanent_failure`, so the report is the same document.
- The three `*_not_observed` gaps are deleted rather than made conditional: the absence of an optional section is not a gap. `AiGapKind` values stay in the contract.
- The parser executor reaches the asset verification that `parse_archive` already has. The executor takes the already-extracted artifacts' verification as an input so `reparse.rs` and `parser_migration.rs` keep one code path.
- The survivor repair runs inside the row-removal transaction, after the deleted exports' observations are known and before they are removed, and stamps its facts one millisecond after the tombstone so the pump publishes them afterwards.

- A conversation is one row per account and provider id, shared by every export that holds it, exactly as a project already was (`UNIQUE (account_id, external_id)` and an upsert that returns the row id). Without it no record is ever held by two exports and the survivor repair has nothing to repair; with it an export that reads a conversation unchanged only observes it (and its messages), and the fact it would announce is the one already queued. This changes which `ai_conversation_id` a second export announces: the first export's.

## Risks

- A recreated broker restarts nothing: message ids are UUIDs, so the two-minute duplicate window no longer depends on a counter.
- Existing development databases must be recreated from the edited `schema.sql`; there are no migrations.
