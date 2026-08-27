## 1. Edge receipt claims

- [x] 1.1 RED: add an HTTP fixture that sends the Platform-minted receipt headers and proves the current service rejects or cannot route it for the right boundary reason.
- [x] 1.2 GREEN: add the loopback receipt route, strict claim parsing, explicit Platform-user-to-account mapping, and raw-byte digest/size verification.

## 2. Terminal operation report

- [x] 2.1 RED: add a receipt completion fixture that expects exactly one bounded `platform.operation.reported.v1` terminal payload for the supplied operation id.
- [x] 2.2 GREEN: persist/publish the terminal report through the ChatGPT outbox and project a real raw-stored/unknown-completeness partial result or a safe terminal failure.

## 3. Verification

- [x] 3.1 Run focused receipt/event tests before implementation; each RED must fail by missing behavior rather than compilation.
- [x] 3.2 Run the repository full gate and OpenSpec validation after both behavior pairs are green.
