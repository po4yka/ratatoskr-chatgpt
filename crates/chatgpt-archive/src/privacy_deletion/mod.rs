//! Tenant-authorized, inventory-first privacy deletion.

mod execution;
mod model;
mod rows;
pub(crate) mod service;

pub use model::{
    DeletionAction, DeletionInventoryItem, DeletionPlan, DeletionReport, DeletionStatus,
    PrivacyDeletionScope,
};
pub use service::{FinalizationFault, PrivacyDeletionError, PrivacyDeletionService};

/// Every foreign key into a deletion root that does not cascade, as `(table, column)`.
///
/// Deletion removes rows from `exports`, `import_runs`, `accounts`, `projects`,
/// `conversations` and `messages`. A foreign key into one of them that is neither
/// `ON DELETE CASCADE` nor named here would make the deletion fail after bytes were
/// at risk, so `tests/privacy_deletion_closure.rs` reads `pg_constraint` and fails on
/// any reference this list does not handle, and on any entry the schema no longer has.
/// Adding a table that references a root means handling it in the rows phase and
/// naming it here in the same change.
pub const HANDLED_REFERENCES: &[(&str, &str)] = &[
    // Removed with their scope, in dependency order.
    ("exports", "account_id"),
    ("projects", "account_id"),
    ("conversations", "account_id"),
    ("import_runs", "export_id"),
    ("completeness_reports", "import_run_id"),
    ("raw_records", "export_id"),
    ("revisions", "observed_in"),
    ("message_relations", "observed_in_export"),
    ("message_relations", "from_message_id"),
    ("message_relations", "to_message_id"),
    ("messages", "conversation_id"),
    ("content_parts", "message_id"),
    // A message and its parent are removed in one statement.
    ("messages", "parent_message_id"),
    // Dependents of an export that are removed with it.
    ("platform_operation_imports", "import_run_id"),
    ("platform_operation_imports", "export_id"),
    ("reparse_runs", "export_id"),
    // Rows that survive because another export still evidences them are repointed to
    // a retained observing export, or to none.
    ("projects", "first_seen_export"),
    ("projects", "last_seen_export"),
    ("conversations", "first_seen_export"),
    ("conversations", "last_seen_export"),
    ("assets", "observed_in"),
    // A surviving conversation never points at a project this deletion removes.
    ("conversations", "project_id"),
];
