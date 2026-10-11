## Decisions

- The classification belongs in the blob store, not in each caller. `read_verified` and `locate` had mapped every failure to `Missing`, so no caller could be correct; they now share one `read_failure` rule: `NotFound` and `NotADirectory` are `Missing`, everything else is `Unavailable`. This matches `erase`, which already reported non-`NotFound` failures as `Unavailable`.
- `ParserExecutionError::Storage` is a unit variant. The error crosses a boundary whose rule is "no source content", and the importer needs only the class. The cause is not lost for operators: the pass returns an error and the pump logs and retries it.
- `is_permanent_import_error` names the permanent parser variant (`Failed`) instead of matching every parser error, so a future variant is retryable until someone decides otherwise.
- Out of scope, recorded: `ArchiveInspector::inspect` and the artifact extractor still map any blob failure to `ArchiveIntakeError::EvidenceUnavailable`, which the importer treats as permanent. After this change a raw-archive verify fault is caught earlier as a retryable `ReparseError::Blob`, so the remaining window is a fault between that verify and the inspect.
- A single-export reparse of an export that is not the newest observer of a record still writes its older content into the shared row (review minor finding, carried as a `TODO(author)` at `persist_projection`).
