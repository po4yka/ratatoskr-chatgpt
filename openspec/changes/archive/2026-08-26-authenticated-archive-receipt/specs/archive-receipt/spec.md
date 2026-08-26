# archive-receipt Specification

## Purpose

Receives ChatGPT export archives from authenticated tenants as streaming uploads, hashes them while they arrive, caps their size, stores the original bytes immutably through the service's content-addressed blob storage, and answers with an explicit stored-or-duplicate outcome — so later import stages always have verified raw evidence.

## ADDED Requirements

### Requirement: Receipt requires an authenticated principal

Every archive receipt SHALL require a bearer credential that authenticates to exactly one owning archive account before any byte of the body is read, and requests without a credential, with an unknown credential, or with a malformed credential header SHALL be rejected with the unauthenticated failure envelope and leave no export rows, no import runs, and no stored objects.

#### Scenario: request without a credential is refused

- **WHEN** `POST /exports` arrives with no `Authorization` header
- **THEN** the response carries the unauthenticated envelope with status 401 and no database rows or stored blobs exist for the attempt

#### Scenario: unknown credential is indistinguishable from a missing one

- **WHEN** `POST /exports` arrives with a well-formed bearer credential that matches no configured tenant token
- **THEN** the response is the same 401 unauthenticated envelope as for a missing credential, and no tenant information is disclosed

#### Scenario: valid credential resolves one tenant account

- **WHEN** `POST /exports` arrives with a configured tenant bearer token
- **THEN** the receipt proceeds bound to that token's archive account, and the stored export records that account as its owner

### Requirement: Receipt streams and hashes without buffering the whole file

Receipt SHALL consume the upload as a chunked stream, updating the SHA-256 digest and writing each chunk to isolated staging storage as it arrives; peak process memory SHALL NOT scale with archive size, and the recorded digest and byte length SHALL match an independent hash of the received bytes.

#### Scenario: chunked upload produces the correct digest

- **WHEN** a client uploads an archive larger than one internal chunk across many stream chunks
- **THEN** the receipt answer reports the SHA-256 hex that equals an independently computed hash of the uploaded bytes and a byte length equal to the total bytes sent

#### Scenario: staged bytes equal the published evidence

- **WHEN** a receipt completes with the stored outcome
- **THEN** reading back the referenced blob through the store verifies its digest and returns bytes identical to what the client sent

### Requirement: Archive size is capped

Receipt SHALL enforce a configurable maximum archive size against both the declared `Content-Length` before the body is consumed and the running byte total while the stream is consumed. An oversized declaration SHALL be refused before storage begins; an overrun discovered mid-stream SHALL abort the transfer, leave no published object and no export row, and end in a durable failed import run.

#### Scenario: declared size over the cap is refused before the body is read

- **WHEN** `POST /exports` declares a `Content-Length` greater than the configured maximum archive size
- **THEN** the response carries the payload-too-large envelope with status 413, the request body is not fully consumed, and no export row, import run, or stored object exists for the attempt

#### Scenario: streamed size bomb aborts at the cap

- **WHEN** the request stream delivers more bytes than the configured maximum without a matching oversized declaration
- **THEN** consumption stops once the cap is exceeded, the response carries the payload-too-large envelope with status 413, no object is published, and the attempt's import run is durably recorded as failed

#### Scenario: truncated stream fails durably without publishing

- **WHEN** the upload stream ends with an error before the client completes the body
- **THEN** no object is published and no export row exists, and the attempt's import run remains durably queryable as failed

### Requirement: Stored archives are immutable raw evidence

A completed stored receipt SHALL have published its bytes through the write-once content-addressed blob storage cited by the fleet `blob-references` contract, and the export row SHALL reference the blob by fleet reference JSON together with its digest, byte length, acquisition mode, and receive time.

#### Scenario: re-receiving identical bytes never rewrites the object

- **WHEN** the same tenant uploads byte-identical archive content twice
- **THEN** both receipts resolve to the same blob reference and the stored object is created exactly once

### Requirement: Duplicate archives answer with explicit outcomes

Receipt SHALL detect duplicates by digest within the owning account. Receiving content whose digest already exists for that account SHALL answer with an explicit duplicate outcome naming the existing export, create no new export row, and leave the original evidence untouched. Receiving different content SHALL answer with a stored outcome and a new export row. Two different accounts MAY hold exports with equal digests; per-account uniqueness SHALL NOT be weakened into global uniqueness.

#### Scenario: identical re-upload answers duplicate and adds no row

- **WHEN** a tenant uploads content whose digest already exists as that tenant's export
- **THEN** the response carries the duplicate outcome with the existing export's identifier, status 200, and the number of export rows for that digest is unchanged

#### Scenario: different content answers stored as a new export

- **WHEN** a tenant uploads content whose digest does not exist for that tenant
- **THEN** the response carries the stored outcome with the new export's identifier, status 201, and a new export row referencing freshly published evidence

#### Scenario: equal digests across tenants stay separate exports

- **WHEN** two different tenant accounts each upload byte-identical content
- **THEN** each account owns its own export row for that digest while the underlying blob object is shared by content addressing

### Requirement: Acquisition mode is explicit on every receipt

Receipt SHALL require an acquisition-mode header naming one of the supported modes, and SHALL refuse requests whose mode is missing or unrecognized with the invalid-request envelope before storing anything.

#### Scenario: missing acquisition header is refused

- **WHEN** `POST /exports` arrives without the acquisition-mode header
- **THEN** the response carries the invalid-request envelope with status 400 and nothing is stored

#### Scenario: unknown acquisition value is refused

- **WHEN** the acquisition-mode header names a value outside the supported set
- **THEN** the response carries the invalid-request envelope with status 400 and nothing is stored

### Requirement: Media type is declared on every receipt

Receipt SHALL require a parsable `type/subtype` media type on the upload, use it as the stored blob's media type, and refuse requests without one using the invalid-request envelope.

#### Scenario: absent media type is refused

- **WHEN** `POST /exports` arrives with no parsable `Content-Type`
- **THEN** the response carries the invalid-request envelope with status 400 and nothing is stored
