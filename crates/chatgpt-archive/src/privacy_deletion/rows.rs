//! Phase A of privacy deletion: remove the rows, leave the bytes.
//!
//! Rows go first, bytes second, completion last (XR-021 CONTRACTS.md section S07).
//! This phase runs in ONE transaction under the tenant advisory lock. It deletes the
//! scoped rows in foreign-key-safe order together with their dependents, repoints the
//! provenance of rows that survive, recomputes which blobs are still referenced by the
//! rows that REMAIN, queues the tombstones, and records `purging`. The durable
//! `privacy_deletion_items` stay: they carry the blob references phase B erases, so at
//! no commit point does a surviving row reference a missing blob and every blob not
//! yet erased is listed in a durable item.

use ratatoskr_ai_archive_contracts::AiArchiveTombstone;
use ratatoskr_identifiers::{BlobRef, CorrelationId, TenantRef, WireTimestamp};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::execution::DurableRequest;
use super::service::{
    FinalizationFault, PrivacyDeletionError, PrivacyDeletionService, lock_tenant,
};
use crate::pg_instant::database_now;
use crate::{NormalizedArchiveEvent, OutboxPlacement};

type Tx<'a> = Transaction<'a, Postgres>;

/// Item state of a tombstone that could not be queued for lack of a Platform owner.
pub(super) const UNBOUND_ERROR_CODE: &str = "downstream_tombstone_unbound";

/// Runs phase A. Returns once the rows are gone and the request is `purging`, or
/// immediately when another executor already completed this phase.
pub(super) async fn remove_rows(
    service: &PrivacyDeletionService,
    durable: &DurableRequest,
    request_id: Uuid,
    evidence_ref: &BlobRef,
    fault: FinalizationFault,
) -> Result<(), PrivacyDeletionError> {
    let mut transaction = service.pool.begin().await?;
    lock_tenant(&mut transaction, durable.tenant_id).await?;
    let status: String = sqlx::query_scalar(
        "SELECT status FROM chatgpt_archive.privacy_deletion_requests WHERE id = $1 FOR UPDATE",
    )
    .bind(request_id)
    .fetch_one(&mut *transaction)
    .await?;
    if status != "planned" {
        transaction.rollback().await?;
        return Ok(());
    }
    // The owner is resolved BEFORE the account row can be deleted.
    let owner = service
        .owners
        .owner_of_account(&mut transaction, durable.tenant_id)
        .await?;

    delete_uuid_category(
        &mut transaction,
        request_id,
        "raw_record",
        "DELETE FROM chatgpt_archive.raw_records WHERE id = ANY($1)",
    )
    .await?;
    if fault == FinalizationFault::AfterFirstRemoval {
        transaction.rollback().await?;
        return Err(PrivacyDeletionError::InjectedFinalization);
    }
    delete_scoped_rows(&mut transaction, request_id, durable.tenant_id).await?;
    let removed_at = database_now(&mut transaction).await?;
    insert_tombstones(
        &mut transaction,
        durable,
        request_id,
        owner,
        evidence_ref,
        removed_at,
    )
    .await?;
    recompute_blob_sharing(&mut transaction, request_id).await?;
    sqlx::query(
        "UPDATE chatgpt_archive.privacy_deletion_requests
         SET status = 'purging', rows_removed_at = now(), evidence_ref = $2
         WHERE id = $1",
    )
    .bind(request_id)
    .bind(serde_json::to_value(evidence_ref)?)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    if fault == FinalizationFault::AfterRowsCommitted {
        return Err(PrivacyDeletionError::InjectedFinalization);
    }
    Ok(())
}

/// Deletes every selected row, dependents first, in foreign-key-safe order.
async fn delete_scoped_rows(
    transaction: &mut Tx<'_>,
    request_id: Uuid,
    tenant_id: Uuid,
) -> Result<(), PrivacyDeletionError> {
    let exports = item_ids(transaction, request_id, "export", "remove").await?;
    delete_export_dependents(transaction, &exports).await?;
    for (category, query) in [
        (
            "completeness_report",
            "DELETE FROM chatgpt_archive.completeness_reports WHERE id = ANY($1)",
        ),
        (
            "message_relation",
            "DELETE FROM chatgpt_archive.message_relations WHERE id = ANY($1)",
        ),
        (
            "content_part",
            "DELETE FROM chatgpt_archive.content_parts WHERE id = ANY($1)",
        ),
        (
            "revision",
            "DELETE FROM chatgpt_archive.revisions WHERE id = ANY($1)",
        ),
        (
            "asset",
            "DELETE FROM chatgpt_archive.assets WHERE id = ANY($1)",
        ),
        (
            "message",
            "DELETE FROM chatgpt_archive.messages WHERE id = ANY($1)",
        ),
        (
            "conversation",
            "DELETE FROM chatgpt_archive.conversations WHERE id = ANY($1)",
        ),
    ] {
        delete_uuid_category(transaction, request_id, category, query).await?;
    }
    // A conversation that survives because another export still evidences it must not
    // point at a project this deletion removes.
    let removed_projects = item_ids(transaction, request_id, "project", "remove").await?;
    sqlx::query(
        "UPDATE chatgpt_archive.conversations SET project_id = NULL
         WHERE project_id = ANY($1)",
    )
    .bind(&removed_projects)
    .execute(&mut **transaction)
    .await?;
    for (category, query) in [
        (
            "project",
            "DELETE FROM chatgpt_archive.projects WHERE id = ANY($1)",
        ),
        (
            "extracted_artifact",
            "DELETE FROM chatgpt_archive.extracted_artifacts WHERE id = ANY($1)",
        ),
        (
            "import_run",
            "DELETE FROM chatgpt_archive.import_runs WHERE id = ANY($1)",
        ),
    ] {
        delete_uuid_category(transaction, request_id, category, query).await?;
    }
    for (category, query) in [
        (
            "inbox_event",
            "DELETE FROM chatgpt_archive.inbox_events WHERE id = ANY($1)",
        ),
        (
            "outbox_event",
            "DELETE FROM chatgpt_archive.outbox_events WHERE id = ANY($1)",
        ),
    ] {
        delete_i64_category(transaction, request_id, category, query).await?;
    }
    repoint_surviving_provenance(transaction, &exports).await?;
    delete_exports_and_account(transaction, request_id, tenant_id, &exports).await
}

/// Platform operation bindings and reparse results depend on the export and its
/// import runs. They are removed by those dependencies, not only by the planned
/// inventory, so a binding created after planning cannot block the export delete.
async fn delete_export_dependents(
    transaction: &mut Tx<'_>,
    exports: &[Uuid],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM chatgpt_archive.platform_operation_imports
         WHERE export_id = ANY($1)
            OR import_run_id IN (SELECT id FROM chatgpt_archive.import_runs
                                 WHERE export_id = ANY($1))",
    )
    .bind(exports)
    .execute(&mut **transaction)
    .await?;
    sqlx::query("DELETE FROM chatgpt_archive.reparse_runs WHERE export_id = ANY($1)")
        .bind(exports)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn delete_exports_and_account(
    transaction: &mut Tx<'_>,
    request_id: Uuid,
    tenant_id: Uuid,
    exports: &[Uuid],
) -> Result<(), PrivacyDeletionError> {
    sqlx::query("DELETE FROM chatgpt_archive.export_entity_observations WHERE export_id = ANY($1)")
        .bind(exports)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("DELETE FROM chatgpt_archive.exports WHERE id = ANY($1)")
        .bind(exports)
        .execute(&mut **transaction)
        .await?;
    let removes_account: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM chatgpt_archive.privacy_deletion_items
         WHERE request_id = $1 AND category = 'account' AND action = 'remove')",
    )
    .bind(request_id)
    .fetch_one(&mut **transaction)
    .await?;
    if removes_account {
        sqlx::query("DELETE FROM chatgpt_archive.accounts WHERE id = $1")
            .bind(tenant_id)
            .execute(&mut **transaction)
            .await?;
    }
    Ok(())
}

/// Repoints the first-seen, last-seen and observed-in provenance of rows that remain
/// away from the exports being deleted.
///
/// A surviving project, conversation or asset is still evidenced by some other
/// export. First-seen and observed-in move to the earliest retained observing export
/// (export ids are `UUIDv7`, so id order is time order) and last-seen to the latest
/// one. A row with no retained observer gets none rather than a dangling reference.
async fn repoint_surviving_provenance(
    transaction: &mut Tx<'_>,
    exports: &[Uuid],
) -> Result<(), sqlx::Error> {
    for (table, kind) in [("projects", "project"), ("conversations", "conversation")] {
        let statement = format!(
            "UPDATE chatgpt_archive.{table} t SET
               first_seen_export = (SELECT o.export_id
                  FROM chatgpt_archive.export_entity_observations o
                  WHERE o.entity_kind = '{kind}' AND o.entity_id = t.id
                    AND NOT (o.export_id = ANY($1)) ORDER BY o.export_id LIMIT 1),
               last_seen_export = (SELECT o.export_id
                  FROM chatgpt_archive.export_entity_observations o
                  WHERE o.entity_kind = '{kind}' AND o.entity_id = t.id
                    AND NOT (o.export_id = ANY($1)) ORDER BY o.export_id DESC LIMIT 1)
             WHERE t.first_seen_export = ANY($1) OR t.last_seen_export = ANY($1)"
        );
        sqlx::query(&statement)
            .bind(exports)
            .execute(&mut **transaction)
            .await?;
    }
    sqlx::query(
        "UPDATE chatgpt_archive.assets a SET observed_in =
           (SELECT o.export_id FROM chatgpt_archive.export_entity_observations o
            WHERE o.entity_kind = 'asset' AND o.entity_id = a.id
              AND NOT (o.export_id = ANY($1)) ORDER BY o.export_id LIMIT 1)
         WHERE a.observed_in = ANY($1)",
    )
    .bind(exports)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

/// Re-decides every blob item against the rows that remain once the scope is gone.
///
/// An item is `retain_shared` when any remaining export, extracted artifact, asset or
/// content part still references the blob, and `erase` otherwise. The plan only
/// predicted this; after the rows are deleted it is a fact.
async fn recompute_blob_sharing(
    transaction: &mut Tx<'_>,
    request_id: Uuid,
) -> Result<(), PrivacyDeletionError> {
    let items: Vec<(i32, serde_json::Value)> = sqlx::query_as(
        "SELECT ordinal, blob_ref FROM chatgpt_archive.privacy_deletion_items
         WHERE request_id = $1 AND blob_ref IS NOT NULL ORDER BY ordinal",
    )
    .bind(request_id)
    .fetch_all(&mut **transaction)
    .await?;
    for (ordinal, blob_ref) in items {
        let (action, state) = if blob_is_referenced(transaction, &blob_ref).await? {
            ("retain_shared", "retained")
        } else {
            ("erase", "planned")
        };
        sqlx::query(
            "UPDATE chatgpt_archive.privacy_deletion_items SET action = $3, state = $4
             WHERE request_id = $1 AND ordinal = $2 AND state <> 'purged'",
        )
        .bind(request_id)
        .bind(ordinal)
        .bind(action)
        .bind(state)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

/// Whether any row that still exists references this blob.
pub(super) async fn blob_is_referenced(
    transaction: &mut Tx<'_>,
    blob_ref: &serde_json::Value,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1 FROM chatgpt_archive.exports WHERE blob_ref = $1
           UNION ALL
           SELECT 1 FROM chatgpt_archive.extracted_artifacts WHERE blob_ref = $1
           UNION ALL
           SELECT 1 FROM chatgpt_archive.assets WHERE blob_ref = $1
           UNION ALL
           SELECT 1 FROM chatgpt_archive.content_parts WHERE blob_ref = $1
         )",
    )
    .bind(blob_ref)
    .fetch_one(&mut **transaction)
    .await
}

/// Queues one tombstone envelope per downstream subject, in the same transaction as
/// the row removal.
///
/// An account with no Platform owner queues nothing: the tombstone would name an
/// owner Knowledge cannot match. The skip is made visible instead of silent. The item
/// is marked failed with a fixed code, and the completion report counts it as
/// `downstream_tombstone_unbound`. It never blocks the deletion.
async fn insert_tombstones(
    transaction: &mut Tx<'_>,
    durable: &DurableRequest,
    request_id: Uuid,
    owner: Option<TenantRef>,
    evidence_ref: &BlobRef,
    observed_at: WireTimestamp,
) -> Result<(), PrivacyDeletionError> {
    let subjects: Vec<String> = sqlx::query_scalar(
        "SELECT opaque_id FROM chatgpt_archive.privacy_deletion_items
         WHERE request_id = $1 AND category = 'downstream_tombstone'
         ORDER BY opaque_id",
    )
    .bind(request_id)
    .fetch_all(&mut **transaction)
    .await?;
    let fallback_archive = subjects
        .iter()
        .filter_map(|subject| subject.strip_prefix("archive:"))
        .find_map(|id| Uuid::parse_str(id).ok());
    let correlation = durable
        .correlation_id
        .map(|id| CorrelationId(id).as_entity_ref());
    for subject in subjects {
        let Some(owner) = owner else {
            mark_tombstone(
                transaction,
                request_id,
                &subject,
                "failed",
                Some(UNBOUND_ERROR_CODE),
            )
            .await?;
            continue;
        };
        let (archive_id, subject_json) = tombstone_subject(&subject, fallback_archive)?;
        let payload: AiArchiveTombstone = serde_json::from_value(serde_json::json!({
            "ai_archive_id": archive_id,
            "provider": "chatgpt",
            "owner": owner,
            "subject": subject_json,
            "reason": "user_requested",
            "evidence_ref": evidence_ref,
            "observed_at": observed_at,
        }))?;
        let mut event = NormalizedArchiveEvent::tombstoned(&payload)?;
        if let Some(correlation) = correlation.clone() {
            event = event.with_correlation(correlation);
        }
        let placement = OutboxPlacement {
            account_id: Some(durable.tenant_id),
            export_id: None,
            deduplication_key: Some(format!("privacy-delete:{request_id}:{subject}")),
        };
        crate::Database::enqueue_normalized_event(transaction, &event, &placement).await?;
        mark_tombstone(transaction, request_id, &subject, "finalized", None).await?;
    }
    Ok(())
}

async fn mark_tombstone(
    transaction: &mut Tx<'_>,
    request_id: Uuid,
    subject: &str,
    state: &str,
    error_code: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE chatgpt_archive.privacy_deletion_items SET state = $3, error_code = $4
         WHERE request_id = $1 AND category = 'downstream_tombstone' AND opaque_id = $2",
    )
    .bind(request_id)
    .bind(subject)
    .bind(state)
    .bind(error_code)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn tombstone_subject(
    subject: &str,
    fallback_archive: Option<Uuid>,
) -> Result<(Uuid, serde_json::Value), PrivacyDeletionError> {
    let parse = |id: &str| Uuid::parse_str(id).map_err(|_| PrivacyDeletionError::InvalidInventory);
    if let Some(id) = subject.strip_prefix("archive:") {
        return Ok((parse(id)?, serde_json::json!({"subject_kind": "archive"})));
    }
    let archive = fallback_archive.ok_or(PrivacyDeletionError::InvalidInventory)?;
    if let Some(id) = subject.strip_prefix("conversation:") {
        return Ok((
            archive,
            serde_json::json!({"subject_kind": "conversation", "ai_conversation_id": parse(id)?}),
        ));
    }
    if let Some(id) = subject.strip_prefix("project:") {
        return Ok((
            archive,
            serde_json::json!({"subject_kind": "project", "ai_project_id": parse(id)?}),
        ));
    }
    Err(PrivacyDeletionError::InvalidInventory)
}

async fn delete_uuid_category(
    transaction: &mut Tx<'_>,
    request_id: Uuid,
    category: &str,
    query: &str,
) -> Result<(), PrivacyDeletionError> {
    let ids = item_ids(transaction, request_id, category, "remove").await?;
    sqlx::query(query)
        .bind(ids)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

/// The identities of one inventory category with one planned action.
async fn item_ids(
    transaction: &mut Tx<'_>,
    request_id: Uuid,
    category: &str,
    action: &str,
) -> Result<Vec<Uuid>, PrivacyDeletionError> {
    let values: Vec<String> = sqlx::query_scalar(
        "SELECT opaque_id FROM chatgpt_archive.privacy_deletion_items
         WHERE request_id = $1 AND category = $2 AND action = $3 ORDER BY opaque_id",
    )
    .bind(request_id)
    .bind(category)
    .bind(action)
    .fetch_all(&mut **transaction)
    .await?;
    values
        .into_iter()
        .map(|value| Uuid::parse_str(&value).map_err(|_| PrivacyDeletionError::InvalidInventory))
        .collect()
}

async fn delete_i64_category(
    transaction: &mut Tx<'_>,
    request_id: Uuid,
    category: &str,
    query: &str,
) -> Result<(), PrivacyDeletionError> {
    let values: Vec<String> = sqlx::query_scalar(
        "SELECT opaque_id FROM chatgpt_archive.privacy_deletion_items
         WHERE request_id = $1 AND category = $2 AND action = 'remove' ORDER BY opaque_id",
    )
    .bind(request_id)
    .bind(category)
    .fetch_all(&mut **transaction)
    .await?;
    let ids: Result<Vec<i64>, _> = values
        .into_iter()
        .map(|value| value.parse::<i64>())
        .collect();
    let ids = ids.map_err(|_| PrivacyDeletionError::InvalidInventory)?;
    sqlx::query(query)
        .bind(ids)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}
