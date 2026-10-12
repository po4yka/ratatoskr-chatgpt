//! The ledger of which owner each subject was announced under (XR-021 CONTRACTS.md R5-06).

use ratatoskr_identifiers::TenantRef;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// The kind of subject a fact announces to Knowledge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubjectKind {
    /// A whole archive, announced by `ai_archive.archive.imported`.
    Archive,
    /// One conversation, announced by `conversation.added|updated`.
    Conversation,
    /// One project, announced by `project.added|updated`.
    Project,
}

/// What an enqueued fact tells Knowledge exists, and under whom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AnnouncedSubject {
    pub(crate) kind: SubjectKind,
    pub(crate) subject_id: Uuid,
    pub(crate) ai_archive_id: Uuid,
    pub(crate) owner: TenantRef,
}

impl SubjectKind {
    /// The `subject_kind` column value.
    pub(crate) const fn as_column(self) -> &'static str {
        match self {
            Self::Archive => "archive",
            Self::Conversation => "conversation",
            Self::Project => "project",
        }
    }
}

/// Records the announcement. A subject announced again under the same owner adds nothing.
pub(crate) async fn record(
    transaction: &mut Transaction<'_, Postgres>,
    subject: &AnnouncedSubject,
    account_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO chatgpt_archive.announced_subjects
         (subject_kind, subject_id, ai_archive_id, owner_ref, account_id)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (subject_kind, subject_id, owner_ref) DO NOTHING",
    )
    .bind(subject.kind.as_column())
    .bind(subject.subject_id.to_string())
    .bind(subject.ai_archive_id)
    .bind(subject.owner.to_string())
    .bind(account_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}
