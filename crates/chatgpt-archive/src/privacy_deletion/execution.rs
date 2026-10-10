//! Orchestration of privacy deletion by durable request status.
//!
//! Rows go first, bytes second, completion last (XR-021 CONTRACTS.md section S07).
//! `planned` runs the content-free evidence blob and phase A (`rows`), `purging` runs
//! phase B (erase the listed bytes) and phase C (audit, completion, drop the items),
//! and `completed` returns the stored report. Every phase is idempotent, so a crash
//! at any point converges when the request is executed again.

use bytes::Bytes;
use futures_util::stream;
use ratatoskr_identifiers::{BlobRef, WireTimestamp};
use sqlx::PgPool;
use uuid::Uuid;

use super::model::{DeletionPlan, DeletionReport, DeletionStatus, PrivacyDeletionScope};
use super::rows::{self, UNBOUND_ERROR_CODE};
use super::service::{
    FinalizationFault, PrivacyDeletionError, PrivacyDeletionService, lock_tenant,
};

/// The category the completion report adds when a tombstone had no Platform owner.
const UNBOUND_TOTAL: &str = "downstream_tombstone_unbound";

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

    /// Executes with a deterministic fault for transaction and resume tests.
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
        match durable.status.as_str() {
            "completed" => stored_report(&durable),
            "planned" => {
                let plan = load_plan(&self.pool, request_id, &durable).await?;
                let evidence_ref = self.store_evidence(&plan).await?;
                rows::remove_rows(self, &durable, request_id, &evidence_ref, fault).await?;
                self.finish(request_id).await
            }
            "purging" => self.finish(request_id).await,
            _ => Err(PrivacyDeletionError::Conflict),
        }
    }

    /// Phase 0: the content-free evidence blob. Its bytes are a function of the
    /// request alone (no timestamp, no content), so a retry stores the same blob.
    async fn store_evidence(&self, plan: &DeletionPlan) -> Result<BlobRef, PrivacyDeletionError> {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "request_id": plan.request_id,
            "scope": plan.scope,
            "totals": plan.totals,
        }))?;
        Ok(self
            .blobs
            .store(
                "application/json",
                stream::iter([Ok::<Bytes, std::io::Error>(Bytes::from(bytes))]),
            )
            .await?)
    }

    /// Phases B and C for a request whose rows are already gone.
    async fn finish(&self, request_id: Uuid) -> Result<DeletionReport, PrivacyDeletionError> {
        self.erase_listed_blobs(request_id).await?;
        self.complete(request_id).await
    }

    /// Phase B: erase each listed blob nothing references any more, then mark it
    /// purged. No long transaction is held across the filesystem work.
    async fn erase_listed_blobs(&self, request_id: Uuid) -> Result<(), PrivacyDeletionError> {
        let durable = load_request(&self.pool, request_id).await?;
        let items: Vec<(i32, serde_json::Value)> = sqlx::query_as(
            "SELECT ordinal, blob_ref FROM chatgpt_archive.privacy_deletion_items
             WHERE request_id = $1 AND blob_ref IS NOT NULL AND action = 'erase'
               AND state <> 'purged' ORDER BY ordinal",
        )
        .bind(request_id)
        .fetch_all(&self.pool)
        .await?;
        for (ordinal, encoded) in items {
            let reference: BlobRef = serde_json::from_value(encoded.clone())?;
            // The tenant lock narrows the window in which a concurrent import of
            // identical bytes could register a new reference between the check and
            // the erase. A cross-tenant import is outside the lock; closing that
            // needs blob reference counts and is a recorded residual risk.
            let mut transaction = self.pool.begin().await?;
            lock_tenant(&mut transaction, durable.tenant_id).await?;
            if rows::blob_is_referenced(&mut transaction, &encoded).await? {
                set_item_state(
                    &mut transaction,
                    request_id,
                    ordinal,
                    "retain_shared",
                    "retained",
                )
                .await?;
            } else {
                self.blobs.erase(&reference).await?;
                set_item_state(&mut transaction, request_id, ordinal, "erase", "purged").await?;
            }
            transaction.commit().await?;
        }
        Ok(())
    }

    /// Phase C: write the audit, complete the request and drop the items, together.
    async fn complete(&self, request_id: Uuid) -> Result<DeletionReport, PrivacyDeletionError> {
        let durable = load_request(&self.pool, request_id).await?;
        let plan = load_plan(&self.pool, request_id, &durable).await?;
        let evidence_ref: BlobRef = serde_json::from_value(
            durable
                .evidence_ref
                .clone()
                .ok_or(PrivacyDeletionError::InvalidInventory)?,
        )?;
        let mut totals = plan.totals;
        let unbound = unbound_tombstones(&self.pool, request_id).await?;
        if unbound > 0 {
            totals.insert(UNBOUND_TOTAL.to_owned(), unbound);
        }
        let report = DeletionReport {
            request_id,
            status: DeletionStatus::Completed,
            totals,
            evidence_ref,
            completed_at: completion_timestamp(WireTimestamp::now()),
        };

        let mut transaction = self.pool.begin().await?;
        lock_tenant(&mut transaction, durable.tenant_id).await?;
        let (status, stored): (String, Option<serde_json::Value>) = sqlx::query_as(
            "SELECT status, completion_report
             FROM chatgpt_archive.privacy_deletion_requests WHERE id = $1 FOR UPDATE",
        )
        .bind(request_id)
        .fetch_one(&mut *transaction)
        .await?;
        if status == "completed" {
            transaction.rollback().await?;
            return stored_report_value(stored);
        }
        sqlx::query(
            "INSERT INTO chatgpt_archive.privacy_deletion_audits
             (id, request_id, tenant_id, scope_kind, category_counts, outcome,
              evidence_ref, correlation_id, completed_at)
             VALUES ($1, $2, $3, $4, $5, 'completed', $6, $7, $8::timestamptz)",
        )
        .bind(Uuid::now_v7())
        .bind(request_id)
        .bind(durable.tenant_id)
        .bind(&durable.scope_kind)
        .bind(serde_json::to_value(&report.totals)?)
        .bind(serde_json::to_value(&report.evidence_ref)?)
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
        .bind(request_id)
        .bind(serde_json::to_value(&report)?)
        .bind(&report.completed_at)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("DELETE FROM chatgpt_archive.privacy_deletion_items WHERE request_id = $1")
            .bind(request_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(report)
    }
}

fn completion_timestamp(timestamp: WireTimestamp) -> String {
    timestamp.to_wire()
}

async fn set_item_state(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    request_id: Uuid,
    ordinal: i32,
    action: &str,
    state: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE chatgpt_archive.privacy_deletion_items SET action = $3, state = $4
         WHERE request_id = $1 AND ordinal = $2",
    )
    .bind(request_id)
    .bind(ordinal)
    .bind(action)
    .bind(state)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

/// How many tombstones were not queued because no Platform owner is configured.
async fn unbound_tombstones(pool: &PgPool, request_id: Uuid) -> Result<u64, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chatgpt_archive.privacy_deletion_items
         WHERE request_id = $1 AND category = 'downstream_tombstone'
           AND state = 'failed' AND error_code = $2",
    )
    .bind(request_id)
    .bind(UNBOUND_ERROR_CODE)
    .fetch_one(pool)
    .await?;
    Ok(u64::try_from(count).unwrap_or_default())
}

pub(super) struct DurableRequest {
    pub(super) tenant_id: Uuid,
    pub(super) scope_kind: String,
    pub(super) scope_id: Option<Uuid>,
    pub(super) status: String,
    pub(super) correlation_id: Option<Uuid>,
    pub(super) completion_report: Option<serde_json::Value>,
    pub(super) evidence_ref: Option<serde_json::Value>,
}

type DurableRequestRow = (
    Uuid,
    String,
    Option<Uuid>,
    String,
    Option<Uuid>,
    Option<serde_json::Value>,
    Option<serde_json::Value>,
);

fn stored_report(durable: &DurableRequest) -> Result<DeletionReport, PrivacyDeletionError> {
    stored_report_value(durable.completion_report.clone())
}

fn stored_report_value(
    report: Option<serde_json::Value>,
) -> Result<DeletionReport, PrivacyDeletionError> {
    serde_json::from_value(report.ok_or(PrivacyDeletionError::InvalidInventory)?)
        .map_err(PrivacyDeletionError::Encode)
}

async fn load_request(
    pool: &PgPool,
    request_id: Uuid,
) -> Result<DurableRequest, PrivacyDeletionError> {
    let row: Option<DurableRequestRow> = sqlx::query_as(
        "SELECT tenant_id, scope_kind, scope_id, status, correlation_id, completion_report,
                evidence_ref
         FROM chatgpt_archive.privacy_deletion_requests WHERE id = $1",
    )
    .bind(request_id)
    .fetch_optional(pool)
    .await?;
    row.map(
        |(
            tenant_id,
            scope_kind,
            scope_id,
            status,
            correlation_id,
            completion_report,
            evidence_ref,
        )| DurableRequest {
            tenant_id,
            scope_kind,
            scope_id,
            status,
            correlation_id,
            completion_report,
            evidence_ref,
        },
    )
    .ok_or(PrivacyDeletionError::Conflict)
}

async fn load_plan(
    pool: &PgPool,
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
