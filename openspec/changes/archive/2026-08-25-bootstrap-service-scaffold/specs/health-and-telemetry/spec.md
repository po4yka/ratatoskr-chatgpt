## Purpose

Serves the admin-plane endpoints that report whether the process is alive, ready to serve, and which build is running, and sets up structured telemetry for the process.

## ADDED Requirements

### Requirement: Liveness reports process aliveness

The service SHALL serve `GET /health/live` returning HTTP 200 with a JSON body whose `state` is `"live"`, without consulting any dependency.

#### Scenario: live answers with no database

- **WHEN** `GET /health/live` is requested while the configured database is unreachable
- **THEN** the response is 200 with `state` equal to `"live"`

### Requirement: Readiness reflects real dependency checks

The service SHALL serve `GET /health/ready` returning 200 with `state` equal to `"ready"` only after every registered readiness check passes; each failing check SHALL be reported by name in the body, and the response SHALL be 503 otherwise.

#### Scenario: ready once the database round trip succeeds

- **WHEN** `GET /health/ready` is requested and a real query against the configured database succeeds
- **THEN** the response is 200 with `state` equal to `"ready"` and no failing checks listed

#### Scenario: not ready names the failed check

- **WHEN** `GET /health/ready` is requested while the database round trip fails
- **THEN** the response is 503 with `state` equal to `"not_ready"` and the failing check listed under `checks`

### Requirement: Version reports the running build

The service SHALL serve `GET /version` returning JSON that identifies the running version and git revision of the binary.

#### Scenario: version identifies the build

- **WHEN** `GET /version` is requested
- **THEN** the response contains the crate version and the git revision the binary was built from

### Requirement: Admin responses are never cached

Every admin-plane response SHALL carry `Cache-Control: no-store`.

#### Scenario: health responses are not cacheable

- **WHEN** any `/health/*` or `/metrics` or `/version` response is produced
- **THEN** its headers include `Cache-Control: no-store`

### Requirement: Telemetry emits structured logs to stdout

The service SHALL initialize a tracing subscriber at startup that writes JSON-formatted log events to stdout, filtered by a configurable directive string, and SHALL shut it down explicitly on exit.

#### Scenario: boot logs one structured startup line

- **WHEN** the service starts successfully
- **THEN** at least one line of valid JSON containing a log level field appears on stdout before shutdown, and no unstructured log lines are emitted
