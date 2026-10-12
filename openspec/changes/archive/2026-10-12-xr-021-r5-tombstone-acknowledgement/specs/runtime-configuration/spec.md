## ADDED Requirements

### Requirement: The tombstone resend interval and cap are declared configuration

`RATATOSKR__TOMBSTONE__RESEND_AFTER_SECONDS` SHALL default to 600 and accept 30 to 86400. `RATATOSKR__TOMBSTONE__RESEND_CAP_SECONDS` SHALL default to 21600 and accept values from the interval up to 604800. A value outside its range, or a cap below the interval, SHALL be a reported violation naming the key and never its value.

#### Scenario: Defaults

- **WHEN** neither key is set
- **THEN** the interval is 600 seconds and the cap 21600 seconds

#### Scenario: Out of range

- **WHEN** the interval is 29 or 86401, or the cap is below the interval or above 604800
- **THEN** loading fails with a violation naming the key
