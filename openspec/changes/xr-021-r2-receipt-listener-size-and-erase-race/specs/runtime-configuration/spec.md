## ADDED Requirements

### Requirement: The shipped operator example loads

`deploy/systemd/chatgpt.conf.example` SHALL load through `Config::from_environment` and SHALL set the operator and receipt addresses, the database URL, the blob and staging roots, the Platform account mapping, the event bus URL and the bus NKey seed path.

#### Scenario: The example cannot rot

- **WHEN** the example is loaded with temporary files substituted for its absolute paths
- **THEN** the operator address is `127.0.0.1:9085`, the receipt address is `127.0.0.1:8096`, and the bus URL and seed path match the documented values
