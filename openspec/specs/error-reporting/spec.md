# error-reporting Specification

## Purpose
Classifies failures into client-safe categories and renders exactly one public error shape so internals, content, and credentials never leak through error responses.

## Requirements

### Requirement: Public errors use one envelope

Every client-visible failure response SHALL carry the fleet error-envelope JSON shape with a stable machine-readable code, a human-safe message, and a retryable flag; the envelope SHALL be constructed by exactly one rendering path in the codebase.

#### Scenario: rejection renders the envelope

- **WHEN** a handler rejects a request with a classified client-visible failure kind
- **THEN** the response status and code come from that kind's static mapping and the body matches the error-envelope shape

### Requirement: Internal faults stay opaque

Failures that are not client-visible classifications SHALL render as a generic server fault with static text; source chains, subsystem details, and provider messages SHALL NOT reach the response body.

#### Scenario: internal error leaks nothing

- **WHEN** a handler fails with an internal error whose source chain contains database detail
- **THEN** the response is a generic server-fault envelope whose message is static text and whose body contains none of the source-chain detail

### Requirement: Panics render static text

A panicking handler SHALL produce a plain-text generic server-fault response rather than a dropped connection or a stack trace.

#### Scenario: panic becomes a plain 500

- **WHEN** a handler panics while serving a request
- **THEN** the client receives a 500 response with static text and the process keeps serving subsequent requests
