use bytes::Bytes;
use futures_util::stream;
use ratatoskr_ai_archive_contracts::AiArchiveTombstone;
use ratatoskr_identifiers::{BlobRef, WireTimestamp};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::model::{DeletionPlan, DeletionReport, DeletionStatus, PrivacyDeletionScope};
use super::service::{
    FinalizationFault, PrivacyDeletionError, PrivacyDeletionService, lock_tenant,
};
use crate::NormalizedArchiveEvent;

impl PrivacyDeletionService {
    /// Executes a request only when it belongs to the authenticated tenant.
    ///
    /// # Errors
    ///
    /// Returns [`PrivacyDeletionError::Conflict`] for both unknown and foreign
    /// request identities.
    pub async fn execute_for_tenant(
        &self,
        tenant_id: Uuid,
        request_id: Uuid,
    ) -> Result<DeletionReport, PrivacyDeletionError> {
        let owned: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM chatgpt_archive.privacy_deletion_requests
             WHERE id = $1 AND tenant_id = $2)",
        )
        .bind(request_id)
        .bind(tenant_id)
        .fetch_one(&self.pool)
        .await?;
        if !owned {
            return Err(PrivacyDeletionError::Conflict);
        }
        self.execute(request_id).await
    }

    /// Executes a persisted request using the production fault-free path.
    ///
    /// # Errors
    ///
    /// Returns [`PrivacyDeletionError`] on persistence, blob, or durable-state
    /// failure.
    pub async fn execute(&self, request_id: Uuid) -> Result<DeletionReport, PrivacyDeletionError> {
        self.execute_with_fault(request_id, FinalizationFault::None)
            .await
    }

    /// Executes with a deterministic finalization fault for transaction tests.
    ///
    /// # Errors
    ///
    /// Returns [`PrivacyDeletionError::InjectedFinalization`] at the requested
    /// fault point.
    pub async fn execute_with_fault(
        &self,
        request_id: Uuid,
        fault: FinalizationFault,
    ) -> Result<DeletionReport, PrivacyDeletionError> {
        let durable = load_request(&self.pool, request_id).await?;
        if durable.status == "completed" {
            return serde_json::from_value(
                durable
                    .completion_report
                    .ok_or(PrivacyDeletionError::InvalidInventory)?,
            )
            .map_err(PrivacyDeletionError::Encode);
        }
        let plan = load_plan(&self.pool, request_id, &durable).await?;
        purge_blobs(self, request_id).await?;

        let completed_at = completion_timestamp(WireTimestamp::now());
        let evidence_bytes = serde_json::to_vec(&serde_json::json!({
            "request_id": request_id,
            "scope": plan.scope,
            "totals": plan.totals,
            "completed_at": completed_at,
        }))?;
        let evidence_ref = self
            .blobs
            .store(
                "application/json",
                stream::iter([Ok::<Bytes, std::io::Error>(Bytes::from(evidence_bytes))]),
            )
            .await?;
        let report = DeletionReport {
            request_id,
            status: DeletionStatus::Completed,
            totals: plan.totals,
            evidence_ref,
            completed_at,
        };
        finalize(self, &durable, &report, fault).await?;
        Ok(report)
    }
}

fn completion_timestamp(timestamp: WireTimestamp) -> String {
    timestamp.to_wire()
}

struct DurableRequest {
    tenant_id: Uuid,
    scope_kind: String,
    scope_id: Option<Uuid>,
    status: String,
    correlation_id: Option<Uuid>,
    completion_report: Option<serde_json::Value>,
}

type DurableRequestRow = (
    Uuid,
    String,
    Option<Uuid>,
    String,
    Option<Uuid>,
    Option<serde_json::Value>,
);

async fn load_request(
    pool: &sqlx::PgPool,
    request_id: Uuid,
) -> Result<DurableRequest, PrivacyDeletionError> {
    let row: Option<DurableRequestRow> = sqlx::query_as(
        "SELECT tenant_id, scope_kind, scope_id, status, correlation_id, completion_report
         FROM chatgpt_archive.privacy_deletion_requests WHERE id = $1",
    )
    .bind(request_id)
    .fetch_optional(pool)
    .await?;
    row.map(
        |(tenant_id, scope_kind, scope_id, status, correlation_id, completion_report)| {
            DurableRequest {
                tenant_id,
                scope_kind,
                scope_id,
                status,
                correlation_id,
                completion_report,
            }
        },
    )
    .ok_or(PrivacyDeletionError::Conflict)
}

async fn load_plan(
    pool: &sqlx::PgPool,
    request_id: Uuid,
    durable: &DurableRequest,
) -> Result<DeletionPlan, PrivacyDeletionError> {
    let scope = match (durable.scope_kind.as_str(), durable.scope_id) {
        ("archive", Some(ai_archive_id)) => PrivacyDeletionScope::Archive { ai_archive_id },
        ("conversation", Some(conversation_id)) => {
            PrivacyDeletionScope::Conversation { conversation_id }
        }
        ("tenant", None) => PrivacyDeletionScope::Tenant,
        _ => return Err(PrivacyDeletionError::InvalidInventory),
    };
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT category, opaque_id, action
         FROM chatgpt_archive.privacy_deletion_items
         WHERE request_id = $1 ORDER BY ordinal",
    )
    .bind(request_id)
    .fetch_all(pool)
    .await?;
    let mut items = Vec::with_capacity(rows.len());
    for (category, opaque_id, action) in rows {
        items.push(super::DeletionInventoryItem {
            category,
            opaque_id,
            action: super::DeletionAction::parse(&action)
                .ok_or(PrivacyDeletionError::InvalidInventory)?,
        });
    }
    Ok(DeletionPlan::new(
        request_id,
        durable.tenant_id,
        scope,
        items,
    ))
}

async fn purge_blobs(
    service: &PrivacyDeletionService,
    request_id: Uuid,
) -> Result<(), PrivacyDeletionError> {
    let rows: Vec<(i32, String, Option<serde_json::Value>)> = sqlx::query_as(
        "SELECT ordinal, action, blob_ref
         FROM chatgpt_archive.privacy_deletion_items
         WHERE request_id = $1 AND blob_ref IS NOT NULL ORDER BY ordinal",
    )
    .bind(request_id)
    .fetch_all(&service.pool)
    .await?;
    for (ordinal, action, encoded) in rows {
        let reference: BlobRef =
            serde_json::from_value(encoded.ok_or(PrivacyDeletionError::InvalidInventory)?)?;
        if fresh_blob_is_shared(service, request_id, &reference).await? {
            sqlx::query(
                "UPDATE chatgpt_archive.privacy_deletion_items
                 SET action = 'retain_shared', state = 'retained'
                 WHERE request_id = $1 AND ordinal = $2",
            )
            .bind(request_id)
            .bind(ordinal)
            .execute(&service.pool)
            .await?;
        } else {
            match action.as_str() {
                "retain_shared" | "erase" => {
                    service.blobs.erase(&reference).await?;
                    update_item_state(&service.pool, request_id, ordinal, "purged").await?;
                }
                _ => return Err(PrivacyDeletionError::InvalidInventory),
            }
        }
    }
    Ok(())
}

async fn fresh_blob_is_shared(
    service: &PrivacyDeletionService,
    request_id: Uuid,
    reference: &BlobRef,
) -> Result<bool, PrivacyDeletionError> {
    let mut transaction = service.pool.begin().await?;
    let tenant_id: Uuid = sqlx::query_scalar(
        "SELECT tenant_id FROM chatgpt_archive.privacy_deletion_requests WHERE id = $1",
    )
    .bind(request_id)
    .fetch_one(&mut *transaction)
    .await?;
    lock_tenant(&mut transaction, tenant_id).await?;
    let selected_exports = uuid_item_ids(&mut transaction, request_id, "export").await?;
    let encoded = serde_json::to_value(reference)?;
    let shared: bool = sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1 FROM chatgpt_archive.exports
           WHERE NOT (id = ANY($1)) AND blob_ref = $2
           UNION ALL
           SELECT 1 FROM chatgpt_archive.extracted_artifacts
           WHERE NOT (export_id = ANY($1)) AND blob_ref = $2
           UNION ALL
           SELECT 1 FROM chatgpt_archive.assets a
           WHERE a.blob_ref = $2 AND NOT EXISTS(
             SELECT 1 FROM chatgpt_archive.privacy_deletion_items i
             WHERE i.request_id = $3 AND i.category = 'asset'
               AND i.opaque_id = a.id::text AND i.action = 'remove'
           )
           UNION ALL
           SELECT 1 FROM chatgpt_archive.content_parts p
           WHERE p.blob_ref = $2 AND NOT EXISTS(
             SELECT 1 FROM chatgpt_archive.privacy_deletion_items i
             WHERE i.request_id = $3 AND i.category = 'content_part'
               AND i.opaque_id = p.id::text AND i.action = 'remove'
           )
         )",
    )
    .bind(&selected_exports)
    .bind(encoded)
    .bind(request_id)
    .fetch_one(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(shared)
}

async fn update_item_state(
    pool: &sqlx::PgPool,
    request_id: Uuid,
    ordinal: i32,
    state: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE chatgpt_archive.privacy_deletion_items SET state = $3
         WHERE request_id = $1 AND ordinal = $2",
    )
    .bind(request_id)
    .bind(ordinal)
    .bind(state)
    .execute(pool)
    .await?;
    Ok(())
}

async fn finalize(
    service: &PrivacyDeletionService,
    durable: &DurableRequest,
    report: &DeletionReport,
    fault: FinalizationFault,
) -> Result<(), PrivacyDeletionError> {
    let mut transaction = service.pool.begin().await?;
    lock_tenant(&mut transaction, durable.tenant_id).await?;
    delete_uuid_category(
        &mut transaction,
        report.request_id,
        "raw_record",
        "DELETE FROM chatgpt_archive.raw_records WHERE id = ANY($1)",
    )
    .await?;
    if fault == FinalizationFault::AfterFirstRemoval {
        transaction.rollback().await?;
        return Err(PrivacyDeletionError::InjectedFinalization);
    }

    delete_selected_rows(&mut transaction, report.request_id, durable.tenant_id).await?;
    insert_tombstones(&mut transaction, durable, report).await?;
    let evidence_json = serde_json::to_value(&report.evidence_ref)?;
    sqlx::query(
        "INSERT INTO chatgpt_archive.privacy_deletion_audits
         (id, request_id, tenant_id, scope_kind, category_counts, outcome,
          evidence_ref, correlation_id, completed_at)
         VALUES ($1, $2, $3, $4, $5, 'completed', $6, $7, $8::timestamptz)",
    )
    .bind(Uuid::now_v7())
    .bind(report.request_id)
    .bind(durable.tenant_id)
    .bind(&durable.scope_kind)
    .bind(serde_json::to_value(&report.totals)?)
    .bind(evidence_json)
    .bind(durable.correlation_id)
    .bind(&report.completed_at)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE chatgpt_archive.privacy_deletion_requests
         SET status = 'completed', completion_report = $2,
             completed_at = $3::timestamptz, error_code = NULL
         WHERE id = $1",
    )
    .bind(report.request_id)
    .bind(serde_json::to_value(report)?)
    .bind(&report.completed_at)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("DELETE FROM chatgpt_archive.privacy_deletion_items WHERE request_id = $1")
        .bind(report.request_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

async fn delete_selected_rows(
    transaction: &mut Transaction<'_, Postgres>,
    request_id: Uuid,
    tenant_id: Uuid,
) -> Result<(), PrivacyDeletionError> {
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
    let exports = uuid_item_ids(transaction, request_id, "export").await?;
    sqlx::query("DELETE FROM chatgpt_archive.export_entity_observations WHERE export_id = ANY($1)")
        .bind(&exports)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("DELETE FROM chatgpt_archive.exports WHERE id = ANY($1)")
        .bind(&exports)
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

async fn insert_tombstones(
    transaction: &mut Transaction<'_, Postgres>,
    durable: &DurableRequest,
    report: &DeletionReport,
) -> Result<(), PrivacyDeletionError> {
    let subjects: Vec<String> = sqlx::query_scalar(
        "SELECT opaque_id FROM chatgpt_archive.privacy_deletion_items
         WHERE request_id = $1 AND category = 'downstream_tombstone'
         ORDER BY opaque_id",
    )
    .bind(report.request_id)
    .fetch_all(&mut **transaction)
    .await?;
    let archive_ids: Vec<Uuid> = subjects
        .iter()
        .filter_map(|subject| subject.strip_prefix("archive:"))
        .filter_map(|id| Uuid::parse_str(id).ok())
        .collect();
    let fallback_archive = archive_ids
        .first()
        .copied()
        .ok_or(PrivacyDeletionError::InvalidInventory)?;
    for subject in subjects {
        let (archive_id, subject_json) = tombstone_subject(&subject, fallback_archive)?;
        let payload: AiArchiveTombstone = serde_json::from_value(serde_json::json!({
            "ai_archive_id": archive_id,
            "provider": "chatgpt",
            "owner": format!("user:{}", durable.tenant_id),
            "subject": subject_json,
            "reason": "user_requested",
            "evidence_ref": report.evidence_ref,
            "observed_at": report.completed_at,
        }))?;
        let event = NormalizedArchiveEvent::tombstoned(payload)?;
        sqlx::query(
            "INSERT INTO chatgpt_archive.outbox_events
             (event_type, aggregate_id, tenant_id, payload, correlation_id, deduplication_key)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (deduplication_key) WHERE deduplication_key IS NOT NULL DO NOTHING",
        )
        .bind(event.event_type)
        .bind(event.aggregate_id)
        .bind(durable.tenant_id)
        .bind(event.payload)
        .bind(durable.correlation_id)
        .bind(format!("privacy-delete:{}:{subject}", report.request_id))
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

fn tombstone_subject(
    subject: &str,
    fallback_archive: Uuid,
) -> Result<(Uuid, serde_json::Value), PrivacyDeletionError> {
    if let Some(id) = subject.strip_prefix("archive:") {
        let id = Uuid::parse_str(id).map_err(|_| PrivacyDeletionError::InvalidInventory)?;
        return Ok((id, serde_json::json!({"subject_kind": "archive"})));
    }
    if let Some(id) = subject.strip_prefix("conversation:") {
        let id = Uuid::parse_str(id).map_err(|_| PrivacyDeletionError::InvalidInventory)?;
        return Ok((
            fallback_archive,
            serde_json::json!({"subject_kind": "conversation", "ai_conversation_id": id}),
        ));
    }
    if let Some(id) = subject.strip_prefix("project:") {
        let id = Uuid::parse_str(id).map_err(|_| PrivacyDeletionError::InvalidInventory)?;
        return Ok((
            fallback_archive,
            serde_json::json!({"subject_kind": "project", "ai_project_id": id}),
        ));
    }
    Err(PrivacyDeletionError::InvalidInventory)
}

async fn delete_uuid_category(
    transaction: &mut Transaction<'_, Postgres>,
    request_id: Uuid,
    category: &str,
    query: &str,
) -> Result<(), PrivacyDeletionError> {
    let ids = uuid_item_ids(transaction, request_id, category).await?;
    sqlx::query(query)
        .bind(ids)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn uuid_item_ids(
    transaction: &mut Transaction<'_, Postgres>,
    request_id: Uuid,
    category: &str,
) -> Result<Vec<Uuid>, PrivacyDeletionError> {
    let values: Vec<String> = sqlx::query_scalar(
        "SELECT opaque_id FROM chatgpt_archive.privacy_deletion_items
         WHERE request_id = $1 AND category = $2 AND action = 'remove' ORDER BY opaque_id",
    )
    .bind(request_id)
    .bind(category)
    .fetch_all(&mut **transaction)
    .await?;
    values
        .into_iter()
        .map(|value| Uuid::parse_str(&value).map_err(|_| PrivacyDeletionError::InvalidInventory))
        .collect()
}

async fn delete_i64_category(
    transaction: &mut Transaction<'_, Postgres>,
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

#[cfg(test)]
mod tests {
    use ratatoskr_identifiers::WireTimestamp;

    use super::completion_timestamp;

    #[test]
    fn completion_timestamp_does_not_pad_fractional_seconds() {
        let timestamp = WireTimestamp::parse("2026-08-27T13:37:00.94Z")
            .expect("the regression instant is canonical");

        assert_eq!(completion_timestamp(timestamp), "2026-08-27T13:37:00.94Z");
    }
}
