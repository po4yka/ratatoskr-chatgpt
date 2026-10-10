## ADDED Requirements

### Requirement: The receipt listener advertises its capability without claims

While the receipt route is mounted, the service SHALL answer `GET /v1/capabilities` with `200`, `application/json` and the document `{"service":"chatgpt","capabilities":["ai_archive.receipt"]}`, requiring no header and no claim. The route SHALL NOT exist when the receipt route is not mounted.

#### Scenario: Capability document without headers

- **WHEN** a client sends `GET /v1/capabilities` with no headers to the receipt router
- **THEN** the answer is `200` with a body equal to the contract capability document for `chatgpt`

### Requirement: Every non-success answer of the receipt routes is an error envelope

Every non-2xx answer of the receipt routes, including a method the route does not accept, SHALL be `application/json` carrying an `ErrorEnvelope` with a non-empty code.

#### Scenario: Wrong method on the Platform receipt path

- **WHEN** a client sends `PUT /v1/ai-archives/receipt`
- **THEN** the answer is `405` with an `ErrorEnvelope` whose code is `chatgpt.request.method_not_allowed`
