## ADDED Requirements

### Requirement: The receipt is served on its own loopback listener

The service SHALL serve `POST /v1/ai-archives/receipt`, `GET /v1/capabilities` and the tenant-token `POST /exports` only on the listener named by `RATATOSKR__RECEIPT__LISTEN_ADDRESS` (default `127.0.0.1:8096`), and only while a database and a staging root are configured. The operator listener SHALL NOT serve them and the receipt listener SHALL NOT serve `/metrics`, `/health/*` or `/version`. Configuration SHALL refuse a non-loopback address and an address equal to the operator address.

#### Scenario: Two planes, two listeners

- **WHEN** the service boots with a database and a staging root
- **THEN** `GET /v1/capabilities` on the receipt port answers `200` and `/metrics`, `/health/ready` and `/version` answer `404` there, while the receipt route and the capability route answer `404` on the operator port

#### Scenario: Receipt address equal to the operator address

- **WHEN** both listen-address keys name the same socket address
- **THEN** loading the configuration fails with a violation naming `RATATOSKR__RECEIPT__LISTEN_ADDRESS`

### Requirement: The Platform receipt accepts exactly application/zip

The Platform receipt SHALL answer `400` with code `chatgpt.request.invalid` for any `Content-Type` value other than the literal `application/zip`, including a missing header, a parameterized value and a different letter case.

#### Scenario: Parameterized media type

- **WHEN** a Platform receipt arrives with `Content-Type: application/zip; charset=binary`
- **THEN** the answer is `400` with an `ErrorEnvelope` whose code is `chatgpt.request.invalid`
