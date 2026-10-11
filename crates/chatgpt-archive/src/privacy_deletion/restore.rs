//! Puts the records a deletion keeps back to what the retained exports hold (XR-021
//! CONTRACTS.md R3-06 and S07).
//!
//! A project or conversation is one shared row that carries the newest export's content, and a
//! conversation's content parts are an append-only history that no export owns. Deleting the
//! export that wrote some of that content must not leave the content behind in the row, the
//! history, the snapshots or the fact announced for the survivor. Each export names the digest
//! of the version it presented, so a record is stale exactly when some version it keeps is named
//! by no retained export. It is then rebuilt from the versions retained exports name, oldest
//! export first, so the row ends as the newest retained export left it.

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::service::PrivacyDeletionError;
use crate::ParserId;
use crate::entity_versions::{
    CONVERSATION, ConversationVersion, PROJECT, ProjectVersion, append_revision,
    set_conversation_digest,
};
use crate::normalized_persist::persist_conversation_detail;

type Tx<'a> = Transaction<'a, Postgres>;

/// One version a retained export presented: the export, the digest and the stored snapshot.
type Held = (Uuid, String, serde_json::Value);

/// Restores every project and conversation of the tenant that keeps content no retained export
/// holds, and drops the versions of records that no longer exist. It runs in the row-removal
/// transaction after the scope's rows and observations are gone.
pub(super) async fn restore_retained_versions(
    transaction: &mut Tx<'_>,
    tenant_id: Uuid,
) -> Result<(), PrivacyDeletionError> {
    sqlx::query(
        "DELETE FROM chatgpt_archive.entity_versions v
         WHERE NOT EXISTS (SELECT 1 FROM chatgpt_archive.projects p
                           WHERE v.entity_kind = 'project' AND p.id = v.entity_id)
           AND NOT EXISTS (SELECT 1 FROM chatgpt_archive.conversations c
                           WHERE v.entity_kind = 'conversation' AND c.id = v.entity_id)",
    )
    .execute(&mut **transaction)
    .await?;
    let stale: Vec<(String, Uuid)> = sqlx::query_as(
        "SELECT DISTINCT v.entity_kind, v.entity_id
         FROM chatgpt_archive.entity_versions v
         WHERE NOT EXISTS (SELECT 1 FROM chatgpt_archive.export_entity_observations o
                           WHERE o.entity_kind = v.entity_kind AND o.entity_id = v.entity_id
                             AND o.content_digest = v.content_digest)
           AND (v.entity_id IN (SELECT id FROM chatgpt_archive.projects WHERE account_id = $1)
                OR v.entity_id IN (SELECT id FROM chatgpt_archive.conversations
                                   WHERE account_id = $1))
         ORDER BY v.entity_kind, v.entity_id",
    )
    .bind(tenant_id)
    .fetch_all(&mut **transaction)
    .await?;
    for (kind, id) in stale {
        let held = retained_versions(transaction, &kind, id).await?;
        // A record no retained export observes keeps whatever it holds: the scope removes it,
        // or it was never observed by an export at all.
        if let Some(newest) = held.last() {
            match kind.as_str() {
                PROJECT => restore_project(transaction, id, newest).await?,
                CONVERSATION => restore_conversation(transaction, tenant_id, id, &held).await?,
                _ => return Err(PrivacyDeletionError::InvalidInventory),
            }
        }
        sqlx::query(
            "DELETE FROM chatgpt_archive.entity_versions v
             WHERE v.entity_kind = $1 AND v.entity_id = $2
               AND NOT EXISTS (SELECT 1 FROM chatgpt_archive.export_entity_observations o
                               WHERE o.entity_kind = v.entity_kind AND o.entity_id = v.entity_id
                                 AND o.content_digest = v.content_digest)",
        )
        .bind(&kind)
        .bind(id)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

/// The versions retained exports presented of one record, oldest export first (by `received_at`,
/// then export id).
async fn retained_versions(
    transaction: &mut Tx<'_>,
    kind: &str,
    id: Uuid,
) -> Result<Vec<Held>, sqlx::Error> {
    sqlx::query_as(
        "SELECT o.export_id, o.content_digest, v.snapshot
         FROM chatgpt_archive.export_entity_observations o
         JOIN chatgpt_archive.exports e ON e.id = o.export_id
         JOIN chatgpt_archive.entity_versions v
           ON v.entity_kind = o.entity_kind AND v.entity_id = o.entity_id
          AND v.content_digest = o.content_digest
         WHERE o.entity_kind = $1 AND o.entity_id = $2
         ORDER BY e.received_at, e.id",
    )
    .bind(kind)
    .bind(id)
    .fetch_all(&mut **transaction)
    .await
}

/// A project holds one state, so it takes the newest retained version.
async fn restore_project(
    transaction: &mut Tx<'_>,
    id: Uuid,
    (_, digest, snapshot): &Held,
) -> Result<(), PrivacyDeletionError> {
    let version: ProjectVersion = serde_json::from_value(snapshot.clone())?;
    sqlx::query(
        "UPDATE chatgpt_archive.projects
         SET title = $2, description = $3, instructions = $4, parser_name = $5,
             parser_version = $6, content_digest = $7, updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(&version.title)
    .bind(&version.description)
    .bind(&version.instructions)
    .bind(&version.parser_name)
    .bind(&version.parser_version)
    .bind(digest)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

/// A conversation's content parts are a history, so they are written again from the retained
/// versions in the order the exports arrived, and the header ends as the newest one left it.
async fn restore_conversation(
    transaction: &mut Tx<'_>,
    tenant_id: Uuid,
    id: Uuid,
    held: &[Held],
) -> Result<(), PrivacyDeletionError> {
    sqlx::query("UPDATE chatgpt_archive.conversations SET project_id = NULL WHERE id = $1")
        .bind(id)
        .execute(&mut **transaction)
        .await?;
    sqlx::query(
        "DELETE FROM chatgpt_archive.content_parts
         WHERE message_id IN (SELECT id FROM chatgpt_archive.messages WHERE conversation_id = $1)",
    )
    .bind(id)
    .execute(&mut **transaction)
    .await?;
    let mut previous: Option<&str> = None;
    for (export_id, digest, snapshot) in held {
        if previous == Some(digest.as_str()) {
            continue;
        }
        previous = Some(digest.as_str());
        let version: ConversationVersion = serde_json::from_value(snapshot.clone())?;
        let project_id: Option<Uuid> = match &version.project_external_id {
            Some(external_id) => {
                sqlx::query_scalar(
                    "SELECT id FROM chatgpt_archive.projects
                     WHERE account_id = $1 AND external_id = $2",
                )
                .bind(tenant_id)
                .bind(external_id)
                .fetch_optional(&mut **transaction)
                .await?
            }
            None => None,
        };
        persist_conversation_detail(
            transaction,
            *export_id,
            id,
            project_id,
            &version.conversation,
            &ParserId {
                name: version.parser_name.clone(),
                version: version.parser_version.clone(),
            },
        )
        .await?;
    }
    if let Some((export_id, digest, snapshot)) = held.last() {
        let version: ConversationVersion = serde_json::from_value(snapshot.clone())?;
        set_conversation_digest(transaction, id, digest).await?;
        append_revision(
            transaction,
            id,
            *export_id,
            digest,
            (&version.parser_name, &version.parser_version),
        )
        .await?;
    }
    Ok(())
}
