//! The versions of the records that cumulative exports share (XR-021 CONTRACTS.md R3-06).
//!
//! A project or conversation is one row per account however many exports contain it, and the row
//! holds the newest content. Each export that observes the record names the digest of the
//! version it presented, and `entity_versions` keeps one snapshot per distinct digest. A privacy
//! deletion that removes the export which wrote the newest content restores the row from the
//! snapshots the retained exports still name, so no content that only a deleted export held
//! survives (see `privacy_deletion::restore`).

use serde::{Deserialize, Serialize};
use sha2::Digest as _;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::ParsedConversation;

type Tx<'a> = Transaction<'a, Postgres>;

/// The observation kinds that carry a version.
pub(crate) const PROJECT: &str = "project";
pub(crate) const CONVERSATION: &str = "conversation";

/// What one export presented of a project: exactly the content of the project row and the parser
/// stamp the row carried when this content was first written.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProjectVersion {
    pub(crate) title: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) instructions: Option<String>,
    pub(crate) parser_name: String,
    pub(crate) parser_version: String,
}

impl ProjectVersion {
    /// The digest of the content, which excludes the parser stamp: a newer parser release that
    /// reads identical content presents the same version.
    pub(crate) fn digest(&self) -> String {
        let content = serde_json::json!([self.title, self.description, self.instructions]);
        hex::encode(sha2::Sha256::digest(content.to_string().as_bytes()))
    }
}

/// What one export presented of a conversation: the parsed conversation itself, the project it
/// belonged to, and the parser that produced it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ConversationVersion {
    pub(crate) conversation: ParsedConversation,
    pub(crate) project_external_id: Option<String>,
    pub(crate) parser_name: String,
    pub(crate) parser_version: String,
}

/// Records that `export_id` observed the record and which version of it the export presented.
pub(crate) async fn observe(
    transaction: &mut Tx<'_>,
    export_id: Uuid,
    kind: &str,
    entity_id: Uuid,
    digest: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO chatgpt_archive.export_entity_observations
         (export_id, entity_kind, entity_id, content_digest) VALUES ($1, $2, $3, $4)
         ON CONFLICT (export_id, entity_kind, entity_id)
         DO UPDATE SET content_digest = EXCLUDED.content_digest",
    )
    .bind(export_id)
    .bind(kind)
    .bind(entity_id)
    .bind(digest)
    .execute(&mut **transaction)
    .await?;
    // The export no longer presents its earlier version. A version no export presents is not
    // evidence of anything, so it goes.
    sqlx::query(
        "DELETE FROM chatgpt_archive.entity_versions v
         WHERE v.entity_kind = $1 AND v.entity_id = $2 AND v.content_digest <> $3
           AND NOT EXISTS (SELECT 1 FROM chatgpt_archive.export_entity_observations o
                           WHERE o.entity_kind = v.entity_kind AND o.entity_id = v.entity_id
                             AND o.content_digest = v.content_digest)",
    )
    .bind(kind)
    .bind(entity_id)
    .bind(digest)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

/// Keeps the snapshot of a version the first time any export presents its digest.
async fn keep_snapshot(
    transaction: &mut Tx<'_>,
    kind: &str,
    entity_id: Uuid,
    digest: &str,
    snapshot: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO chatgpt_archive.entity_versions
         (entity_kind, entity_id, content_digest, snapshot) VALUES ($1, $2, $3, $4)
         ON CONFLICT DO NOTHING",
    )
    .bind(kind)
    .bind(entity_id)
    .bind(digest)
    .bind(snapshot)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

/// Keeps a project's version. The project row already holds this content.
///
/// # Errors
///
/// Returns the database or encoding error when a row cannot be written.
pub(crate) async fn keep_project_version(
    transaction: &mut Tx<'_>,
    project_id: Uuid,
    version: &ProjectVersion,
) -> Result<(), VersionError> {
    keep_snapshot(
        transaction,
        PROJECT,
        project_id,
        &version.digest(),
        serde_json::to_value(version)?,
    )
    .await?;
    Ok(())
}

/// Keeps a conversation's version and makes the conversation row name it as its current content.
///
/// # Errors
///
/// Returns the database or encoding error when a row cannot be written.
pub(crate) async fn keep_conversation_version(
    transaction: &mut Tx<'_>,
    conversation_id: Uuid,
    digest: &str,
    version: &ConversationVersion,
) -> Result<(), VersionError> {
    keep_snapshot(
        transaction,
        CONVERSATION,
        conversation_id,
        digest,
        serde_json::to_value(version)?,
    )
    .await?;
    set_conversation_digest(transaction, conversation_id, digest).await?;
    Ok(())
}

/// Names the content a conversation row holds now.
pub(crate) async fn set_conversation_digest(
    transaction: &mut Tx<'_>,
    conversation_id: Uuid,
    digest: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE chatgpt_archive.conversations SET content_digest = $2 WHERE id = $1")
        .bind(conversation_id)
        .bind(digest)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

/// Appends a revision of a conversation to its history when `digest` is not already the latest.
pub(crate) async fn append_revision(
    transaction: &mut Tx<'_>,
    conversation_id: Uuid,
    export_id: Uuid,
    digest: &str,
    parser: (&str, &str),
) -> Result<(), sqlx::Error> {
    let latest: Option<String> = sqlx::query_scalar(
        "SELECT payload ->> 'digest' FROM chatgpt_archive.revisions
         WHERE entity_table = 'conversations' AND entity_id = $1
         ORDER BY revision_number DESC LIMIT 1",
    )
    .bind(conversation_id)
    .fetch_optional(&mut **transaction)
    .await?
    .flatten();
    if latest.as_deref() == Some(digest) {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO chatgpt_archive.revisions
         (id, entity_table, entity_id, revision_number, observed_in, payload)
         VALUES ($1, 'conversations', $2,
                 (SELECT COALESCE(max(revision_number), 0) + 1
                  FROM chatgpt_archive.revisions
                  WHERE entity_table = 'conversations' AND entity_id = $2),
                 $3, $4)",
    )
    .bind(Uuid::now_v7())
    .bind(conversation_id)
    .bind(export_id)
    .bind(serde_json::json!({
        "digest": digest,
        "parser_name": parser.0,
        "parser_version": parser.1,
    }))
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

/// A version could not be written or read.
#[derive(Debug, thiserror::Error)]
pub(crate) enum VersionError {
    /// Owned persistence failed.
    #[error("version persistence failed")]
    Store(#[from] sqlx::Error),
    /// A snapshot could not be encoded or decoded.
    #[error("version snapshot encoding failed")]
    Encode(#[from] serde_json::Error),
}
