## Purpose

Loads the service's runtime configuration from the environment into typed structures, failing closed before any listener starts.

## ADDED Requirements

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
