# runtime-configuration Specification

## Purpose
Loads the service's runtime configuration from the environment into typed structures, failing closed before any listener starts.

## Requirements

### Requirement: Configuration loads from prefixed environment variables

The service SHALL read its configuration exclusively from environment variables prefixed `RATATOSKR__`, where a double underscore separates nesting levels, into typed configuration structures that reject unknown keys.

#### Scenario: minimal valid environment boots to typed config

- **WHEN** the process starts with only the required variables set (for example `RATATOSKR__SERVICE__NAME`, `RATATOSKR__DATABASE__URL`)
- **THEN** configuration parses into typed structures with documented defaults applied for every unset optional field, and startup continues

#### Scenario: unknown variable fails startup

- **WHEN** an environment variable carrying the `RATATOSKR__` prefix names no known configuration key (for example `RATATOSKR__SERVICE__NMAE`)
- **THEN** startup stops with exit code 78 and the diagnostic names the offending key without printing any configured value

### Requirement: Validation collects every violation

The service SHALL validate the parsed configuration as one step that reports every violated rule at once, never just the first.

#### Scenario: two bad values report together

- **WHEN** the environment sets a zero connection-pool size and an unparsable bind address at the same time
- **THEN** the failure output lists both violations before the process exits

### Requirement: Secrets are redacted in diagnostics

Secret-valued configuration fields SHALL NOT appear in logs, error messages, or debug output; they render as a fixed placeholder.

#### Scenario: database URL is not echoed

- **WHEN** configuration containing a password-bearing database URL is rendered in any diagnostic or log line during startup
- **THEN** the secret material is replaced by a placeholder and the URL's credentials do not appear

### Requirement: Configuration is finite

Every configuration key the service reads SHALL be declared in code with a type and default documentation; there SHALL be no free-form passthrough of extra keys.

#### Scenario: declared key inventory is enumerable

- **WHEN** the configuration module is inspected
- **THEN** the complete set of accepted keys is visible in the source as typed struct definitions, with no catch-all map of arbitrary keys

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

### Requirement: The operator listener defaults to 9085

The operator listener SHALL default to `127.0.0.1:9085` so it does not collide with the Threads and Claude operator listeners.

#### Scenario: Default listener

- **WHEN** configuration loads with no listener override
- **THEN** the operator listen address is `127.0.0.1:9085`

### Requirement: The shipped operator example loads

`deploy/systemd/chatgpt.conf.example` SHALL load through `Config::from_environment` and SHALL set the operator and receipt addresses, the database URL, the blob and staging roots, the Platform account mapping, the event bus URL and the bus NKey seed path.

#### Scenario: The example cannot rot

- **WHEN** the example is loaded with temporary files substituted for its absolute paths
- **THEN** the operator address is `127.0.0.1:9085`, the receipt address is `127.0.0.1:8096`, and the bus URL and seed path match the documented values

### Requirement: The operator example names a provisioned role

The role and database named by `deploy/systemd/chatgpt.conf.example` SHALL be the ones `deploy/postgres/role.sql` creates: role `ratatoskr_chatgpt` on database `ratatoskr`, port 5432.

#### Scenario: Example and role file agree

- **WHEN** the example's database URL is compared with `deploy/postgres/role.sql`
- **THEN** the role, the database and the port match

### Requirement: The tombstone resend interval and cap are declared configuration

`RATATOSKR__TOMBSTONE__RESEND_AFTER_SECONDS` SHALL default to 600 and accept 30 to 86400. `RATATOSKR__TOMBSTONE__RESEND_CAP_SECONDS` SHALL default to 21600 and accept values from the interval up to 604800. A value outside its range, or a cap below the interval, SHALL be a reported violation naming the key and never its value.

#### Scenario: Defaults

- **WHEN** neither key is set
- **THEN** the interval is 600 seconds and the cap 21600 seconds

#### Scenario: Out of range

- **WHEN** the interval is 29 or 86401, or the cap is below the interval or above 604800
- **THEN** loading fails with a violation naming the key
