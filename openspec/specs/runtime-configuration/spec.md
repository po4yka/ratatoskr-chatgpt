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

The closed configuration key set SHALL include `RATATOSKR__LIMITS__MAX_ARCHIVE_BYTES` (a positive integer byte cap with a documented default), `RATATOSKR__STORAGE__RECEIPT_STAGING_ROOT` (an optional absolute directory path for isolated receipt staging), and `RATATOSKR__RECEIPT__TENANT_TOKENS` (an optional comma-separated list of `<token>=<external-ref>` tenant credentials held as secrets). Every rule violation SHALL be reported through the existing all-violations, value-free diagnostics, and token material SHALL never appear in logs or debug output.

#### Scenario: oversized archive cap value is reported value-free

- **WHEN** the environment sets `RATATOSKR__LIMITS__MAX_ARCHIVE_BYTES` to a non-positive or non-integer value
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
