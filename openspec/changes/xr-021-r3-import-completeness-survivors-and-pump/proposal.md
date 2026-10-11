## Why

Round 3 of changeset XR-021 fixes the defects that the Round 2 native smokes and the independent verifiers found and that were re-verified against `origin/main`. Eight of them land in the ChatGPT archive service. The binding text is XR-021 CONTRACTS.md sections R3-01, R3-03, R3-04, R3-05, R3-06, R3-07, R3-11, R3-16 and R3-19 (Round 3 amendments); this proposal cites them and does not restate them.

- The event pump has no inbox prefix of its own, so once the deployed ACL narrows each identity to `_INBOX_<name>.>` its JetStream requests time out, and until then it can read other services' deliveries (R3-01).
- The pump opens and closes a broker connection on every pass, even with an empty outbox, and names its messages by the bare outbox row number (R3-07 rules 1 and 4).
- A Platform user that is not configured answers 401, which makes the export agent remove its pairing after a full upload; refused receipts and failed imports leave no log line; the capability probe has no `cache-control: no-store` (R3-03 rule 3, R3-07 rules 2 and 3).
- A second upload of bytes whose import failed permanently binds to the failed run and is never reported (R3-04 rule 2).
- Every import is `structurally_partial` because three optional sections are reported as gaps, and an export with `assets.json` fails (R3-05).
- Deleting the export that first announced a record makes Knowledge forget conversations a retained export still holds (R3-06).
- No checked-in file provisions the PostgreSQL role, the contracts pin differs from the rest of the fleet, and three paragraphs describe a state that no longer holds (R3-11, R3-16, R3-19).

## What Changes

- Move every `ratatoskr-*` git dependency to contracts commit `ad16855c4e7f3d52cd118274faa3b8f3ab4da576`.
- One connection function applies the inbox prefix `_INBOX_chatgpt`. `deploy/nats/identity.conf` carries the service's ACL stanza and a test runs the pump against a broker that holds only that stanza. CI puts `nats-server` on `PATH` so that test runs.
- `ArchiveEventOutbox` holds one lazily created client, an empty pass touches no network, a closed client is replaced, and `Nats-Msg-Id` is the envelope `event_id`.
- An unmapped Platform user answers 403 `chatgpt.account.unmapped`. Every refused Platform receipt and every permanently failed import is logged at WARN with a closed class. `GET /v1/capabilities` carries `cache-control: no-store`.
- `process_pending_once` reports a `failed` run that has a bound unreported operation.
- The three `*_not_observed` gaps are no longer produced and the registered parser verifies `assets.json` through the blob store.
- The row-removal transaction of a privacy deletion re-announces the records a retained export still holds and removes the deleted exports' `archive.imported` heads.
- Add `deploy/postgres/role.sql` and a README deployment paragraph; rewrite the stale status paragraphs.

Breaking, with no shims: the pump's `Nats-Msg-Id` key space changes from row numbers to event ids; a service built before this round cannot complete JetStream requests under the Round 3 ACL (deploy the services of one snapshot together); an unmapped Platform user is answered 403 instead of 401; the three `*_not_observed` gaps disappear from completeness reports; an export that carries `assets.json` is imported instead of refused.

## Impact

Touches the event pump, receipt HTTP and its error vocabulary, the importer and its selector, the parser executor, the contract projection, privacy deletion, `Cargo.toml`/`Cargo.lock`, CI, `deploy/` and documentation. The explicit `downstream_tombstone_unbound: 0`, the content-free evidence blob, the account-erasure transport and Platform's side of the receipt are out of scope.
