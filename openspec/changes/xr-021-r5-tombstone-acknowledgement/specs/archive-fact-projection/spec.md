## ADDED Requirements

### Requirement: A tombstone is re-sent until Knowledge acknowledges it

When the pump marks a tombstone published it SHALL set `next_resend_at` to now plus the configured resend interval. A published tombstone with no `acknowledged_at` whose `next_resend_at` has passed SHALL be published again with the stored bytes unchanged and the same `Nats-Msg-Id`, its `resend_count` SHALL increase by one and its `next_resend_at` SHALL become now plus the lesser of the interval times two to the old count and the configured cap. A failed republish SHALL leave the row and use the failure backoff. The resend SHALL stop only when `acknowledged_at` is set.

#### Scenario: No acknowledgement arrives

- **WHEN** a tombstone was published and the resend interval passes without an acknowledgement
- **THEN** the same bytes are published again with the same `Nats-Msg-Id` and the next resend waits at most twice as long, never longer than the cap

#### Scenario: The acknowledgement arrives

- **WHEN** Knowledge publishes an `applied` acknowledgement naming the tombstone's event id and owner
- **THEN** `acknowledged_at` is set and the row is never published again

#### Scenario: A refusal

- **WHEN** Knowledge publishes a `refused` acknowledgement
- **THEN** the resend stops, an ERROR line is logged and `chatgpt_archive_tombstone_refused_total` increases

#### Scenario: An acknowledgement that names no row

- **WHEN** an acknowledgement names an unknown event id or another owner's row
- **THEN** no row changes and the message is acknowledged to the broker

#### Scenario: Knowledge is down

- **WHEN** no acknowledgement ever arrives
- **THEN** the service stays ready and the unacknowledged-tombstone gauges grow
