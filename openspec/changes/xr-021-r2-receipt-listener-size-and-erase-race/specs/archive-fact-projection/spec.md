## ADDED Requirements

### Requirement: A fact over the event limit is not enqueued and the import says so

A fact whose serialized envelope is larger than `RATATOSKR__RECEIPT__MAX_EVENT_BYTES` SHALL NOT be enqueued. The terminal operation report written in the same transaction SHALL be `partially_succeeded` with the warning `ai_archive.facts.too_large`, in addition to the one `ai_archive.import.incomplete` warning when the import is also incomplete.

#### Scenario: One oversized conversation

- **WHEN** an import contains a conversation whose `ai_archive.conversation.added.v1` envelope exceeds the configured limit and another that does not
- **THEN** only the smaller conversation fact is enqueued and the report is `partially_succeeded` with the too-large warning and passes `validate()`

### Requirement: An undeliverable outbox row never wedges the pump

A row whose body is larger than the connected broker `max_payload` minus 1 KiB SHALL be marked undeliverable with `last_error = 'payload_too_large'`, SHALL be skipped from then on, and SHALL NOT make the pass fail or the service not ready. A transient refusal SHALL never mark a row undeliverable.

#### Scenario: Oversized row between two small rows

- **WHEN** the pump runs against a broker with a small `max_payload` and the middle row is larger than the limit
- **THEN** the first and last rows are published, the middle row is undeliverable, the pass returns success and readiness stays true
