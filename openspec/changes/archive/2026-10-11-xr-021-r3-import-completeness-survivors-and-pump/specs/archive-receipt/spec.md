## ADDED Requirements

### Requirement: An unmapped Platform user is forbidden, not unauthenticated

The Platform receipt SHALL answer `403` with an `ErrorEnvelope` whose code is `chatgpt.account.unmapped`, `retryable` false and message "This user is not set up for archive import." when the claims parse but the Platform user is not in the configured account mapping. A missing or malformed claim set and an `Authorization` header SHALL stay `401`.

#### Scenario: Unmapped user

- **WHEN** a Platform receipt carries well-formed claims for a user that is not in `PLATFORM_ACCOUNTS`
- **THEN** the answer is `403` with code `chatgpt.account.unmapped` and `retryable` false

#### Scenario: Malformed claims

- **WHEN** a Platform receipt carries a claim that is not a UUID
- **THEN** the answer is `401` with code `chatgpt.auth.unauthenticated`

### Requirement: Every refused Platform receipt is logged with a closed class

The service SHALL log one WARN line for each Platform receipt it refuses with 400, 401, 403, 405 or 413. The line carries a closed class and never a header value, a path or a body byte.

#### Scenario: A refusal leaves one line

- **WHEN** a Platform receipt is refused for any of the five statuses
- **THEN** exactly one WARN event is emitted whose class names the refusal and that carries none of the request's header values

### Requirement: The capability response is not cacheable

`GET /v1/capabilities` SHALL carry `cache-control: no-store`.

#### Scenario: Capability header

- **WHEN** the capability route answers
- **THEN** the response has `cache-control: no-store`
