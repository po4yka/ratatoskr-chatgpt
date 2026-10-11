## ADDED Requirements

### Requirement: An import can be complete

A gap SHALL name something the export contains or references and the importer could not store or model: an `unrecognized_record` or a `missing_file`. The absence of projects, canvas or assets SHALL NOT be a gap. An export with only conversations SHALL be `complete` and its operation `succeeded`.

#### Scenario: Conversations only

- **WHEN** an export holds only `conversations.json`
- **THEN** its completeness is `complete`, it reports no gap and the operation report is `succeeded`

#### Scenario: Missing declared file

- **WHEN** `assets.json` declares an asset whose file is not in the archive
- **THEN** the import records one `missing_file` gap and the operation is `partially_succeeded` with exactly one `ai_archive.import.incomplete` warning

### Requirement: The registered parser imports declared assets

The registered parser path SHALL verify the declarations of `assets.json` against the extracted artifacts and the blob store and store each verified asset. A malformed `assets.json` SHALL fail the import permanently with `chatgpt.archive.invalid`.

#### Scenario: Every declared file present

- **WHEN** `assets.json` declares assets and every file is present
- **THEN** the assets are imported and no gap is recorded

#### Scenario: Malformed assets.json

- **WHEN** `assets.json` is not a valid declaration document
- **THEN** the run ends `failed` with `chatgpt.archive.invalid`
