## 1. Schema (declarative, so no failing test can precede it)

- [x] 1.1 Add `entity_versions` and the `content_digest` columns of `projects`, `conversations` and `export_entity_observations` to `schema.sql` in place.

## 2. A survivor holds only retained content (R3-06, S07)

- [x] 2.1 RED: `tests/privacy_deletion_survivor_content.rs` `a_conversation_edited_by_the_deleted_export_is_restored_to_the_retained_text`, `a_project_edited_by_the_deleted_export_is_restored_to_the_retained_title`, `an_intermediate_version_does_not_survive_between_two_retained_ones`, `deleting_the_newest_export_falls_back_to_the_newest_retained_version` and `a_message_only_the_deleted_export_held_goes_even_after_unchanged_reads` fail because the edited text is still in `content_parts`, the headers and the queued facts (occurrence counts 1 and 2 instead of 0, and 2 instead of 0 for the dropped message); `a_retained_export_that_repeats_the_edit_keeps_it_and_is_named_as_its_evidence` fails because the survivor fact names the earlier export.
- [x] 2.2 GREEN: versions are recorded on every observation, the importer and reparse write them, the rows phase restores stale records before announcing survivors, a survivor fact names the earliest retained export that presented the current content, an unchanged read observes only the messages it presented, and reparse compares the export's own version digest.
- [x] 2.3 `deleting_every_export_leaves_no_version_snapshot` was written with the GREEN step (a count of snapshots exists only once the implementation writes them), so it was not run red.

## 3. Documentation

- [x] 3.1 README: the privacy-deletion paragraph describes versions and the restore, and the event pump paragraph says that an empty pass reads no seed and opens no connection, so a missing seed or an unreachable broker first shows when the first row is queued.

## 4. Gate

- [x] 4.1 The full gate passed locally against a private PostgreSQL 17 and JetStream broker: `cargo fmt --all -- --check`, clippy with `-D warnings`, the 850-line check, `cargo build` and `cargo test --workspace --locked`, `cargo deny check`, `openspec validate --all --strict` and `openspec validate --archived`.
