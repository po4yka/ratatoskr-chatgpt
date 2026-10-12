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
    /// The kind a deletion inventory subject (`archive:<id>`, `conversation:<id>`,
    /// `project:<id>`) names, and its identity.
    pub(crate) fn parse_inventory_subject(subject: &str) -> Option<(Self, Uuid)> {
        let (prefix, id) = subject.split_once(':')?;
        let kind = match prefix {
            "archive" => Self::Archive,
            "conversation" => Self::Conversation,
            "project" => Self::Project,
            _ => return None,
        };
        Some((kind, Uuid::parse_str(id).ok()?))
    }

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

/// The owners one subject was announced under, each with the archive it was announced under, in
/// a stable order. Empty for a subject that was never announced.
pub(crate) async fn owners_of(
    transaction: &mut Transaction<'_, Postgres>,
    account_id: Uuid,
    kind: SubjectKind,
    subject_id: Uuid,
) -> Result<Vec<(TenantRef, Uuid)>, sqlx::Error> {
    let rows: Vec<(String, Uuid)> = sqlx::query_as(
        "SELECT owner_ref, ai_archive_id FROM chatgpt_archive.announced_subjects
         WHERE account_id = $1 AND subject_kind = $2 AND subject_id = $3
         ORDER BY owner_ref",
    )
    .bind(account_id)
    .bind(kind.as_column())
    .bind(subject_id.to_string())
    .fetch_all(&mut **transaction)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(owner, archive)| Some((TenantRef::parse(&owner).ok()?, archive)))
        .collect())
}

/// Deletes the rows of one subject, which a tombstone has just retracted.
pub(crate) async fn forget(
    transaction: &mut Transaction<'_, Postgres>,
    account_id: Uuid,
    kind: SubjectKind,
    subject_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM chatgpt_archive.announced_subjects
         WHERE account_id = $1 AND subject_kind = $2 AND subject_id = $3",
    )
    .bind(account_id)
    .bind(kind.as_column())
    .bind(subject_id.to_string())
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

/// The owner the archive was most recently announced under, when it was announced.
pub(crate) async fn owner_of_archive(
    transaction: &mut Transaction<'_, Postgres>,
    ai_archive_id: Uuid,
) -> Result<Option<TenantRef>, sqlx::Error> {
    let owner: Option<String> = sqlx::query_scalar(
        "SELECT owner_ref FROM chatgpt_archive.announced_subjects
         WHERE subject_kind = 'archive' AND subject_id = $1
         ORDER BY announced_at DESC, owner_ref DESC LIMIT 1",
    )
    .bind(ai_archive_id.to_string())
    .fetch_optional(&mut **transaction)
    .await?;
    Ok(owner.and_then(|raw| TenantRef::parse(&raw).ok()))
}
