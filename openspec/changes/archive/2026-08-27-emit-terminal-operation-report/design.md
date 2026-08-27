## Context

The Platform receipt path strips all caller credentials and injects only minted claims. Existing
`POST /exports` uses a service-local tenant token, so it cannot be reused as an implicit trust
boundary. The receipt already stores immutable bytes and returns a durable export identity.

## Decisions

- Serve `POST /v1/ai-archives/receipt` only on the configured loopback listener. Require every
  Platform-minted header and reject direct or incomplete calls before touching bytes.
- Add an explicit mapping from Platform user UUID to the service-owned account reference. It is
  configuration/persistence metadata, not a new cross-service identity authority.
- Treat successful raw storage as a truthful partial terminal outcome unless a real parser report
  is available: `partially_succeeded`, completeness `unknown`, and a bounded warning that parsing
  has not established complete normalized coverage. A receipt/storage failure reports `failed`.
- Publish through the existing outbox/inbox discipline so a process crash cannot make stored bytes
  look terminally reported without a durable event.

## Risks

- A receiver stores bytes but the event publisher is unavailable: the durable outbox retries; no
  success is fabricated.
- Duplicate Platform delivery: operation id is the idempotence identity for terminal reports.
- A malformed forwarded claim: no account lookup or archive storage occurs.
