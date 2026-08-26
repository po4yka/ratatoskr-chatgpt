## MODIFIED Requirements

### Requirement: Archive receipt limits and locations are declared configuration

The closed configuration key set SHALL include `RATATOSKR__LIMITS__MAX_ARCHIVE_BYTES` (a positive integer byte cap with a documented default), `RATATOSKR__LIMITS__MAX_ARCHIVE_ENTRIES` (a positive entry-count cap), `RATATOSKR__LIMITS__MAX_ARCHIVE_ENTRY_BYTES` (a positive decompressed per-entry byte cap), `RATATOSKR__LIMITS__MAX_ARCHIVE_DECOMPRESSED_BYTES` (a positive aggregate decompressed byte cap), `RATATOSKR__LIMITS__MAX_ARCHIVE_COMPRESSION_RATIO` (a positive per-entry ratio cap), `RATATOSKR__STORAGE__RECEIPT_STAGING_ROOT` (an optional absolute directory path for isolated receipt staging), and `RATATOSKR__RECEIPT__TENANT_TOKENS` (an optional comma-separated list of `<token>=<external-ref>` tenant credentials held as secrets). Every rule violation SHALL be reported through the existing all-violations, value-free diagnostics, and token material SHALL never appear in logs or debug output.

#### Scenario: oversized archive cap value is reported value-free

- **WHEN** the environment sets `RATATOSKR__LIMITS__MAX_ARCHIVE_BYTES` to a non-positive or non-integer value
- **THEN** startup fails with a violation naming that key and the rule broken, without printing the supplied value

#### Scenario: non-positive extraction cap is reported value-free

- **WHEN** the environment sets any archive extraction limit to a non-positive or non-integer value
- **THEN** startup fails with a violation naming that key and the rule broken, without printing the supplied value

#### Scenario: relative staging root is refused

- **WHEN** the environment sets `RATATOSKR__STORAGE__RECEIPT_STAGING_ROOT` to a relative path
- **THEN** startup fails with a violation naming the key and requiring an absolute directory path

#### Scenario: tenant tokens render redacted in diagnostics

- **WHEN** configuration carrying tenant tokens is rendered in debug output
- **THEN** every token's secret material is replaced by a fixed placeholder

#### Scenario: unset staging root leaves receipt unserved but boots the service

- **WHEN** the process starts with no `RATATOSKR__STORAGE__RECEIPT_STAGING_ROOT` configured
- **THEN** the service boots and serves its admin plane normally, and the archive receipt surface is not mounted

#### Scenario: malformed tenant token entry is rejected with the key named

- **WHEN** `RATATOSKR__RECEIPT__TENANT_TOKENS` contains an entry without the `=` separator or an empty token or reference
- **THEN** startup fails listing one violation per malformed entry, naming only keys and rules
