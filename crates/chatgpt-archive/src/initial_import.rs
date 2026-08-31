//! Restart-safe initial import execution and terminal operation correlation.

use std::collections::BTreeMap;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::stream;
use ratatoskr_event_envelope::{EventEnvelope, EventPayload};
use ratatoskr_identifiers::{BlobRef, EntityRef, EventId, WireTimestamp};
use sqlx::PgPool;
use uuid::Uuid;

use crate::privacy_deletion::service::lock_tenant;
use crate::reparse::{
    ReparseError, ReparsePlan, compare, current_projection, persist_artifacts, persist_projection,
    read_artifacts,
};
use crate::{
    AcquisitionMode, ArchiveInspector, ArchiveLimits, BlobStore, ParsedConversations,
    ParserExecutionInput, ParserRegistry,
};

/// Restart-safe executor for raw archives whose receipt transaction reached `stored`.
#[derive(Debug, Clone)]
pub struct InitialImportWorker {
    pool: PgPool,
    blobs: BlobStore,
    registry: Arc<ParserRegistry>,
    limits: ArchiveLimits,
}

impl InitialImportWorker {
    /// Creates a worker over process-owned durable dependencies.
    #[must_use]
    pub fn new(
        pool: PgPool,
        blobs: BlobStore,
        registry: Arc<ParserRegistry>,
        limits: ArchiveLimits,
    ) -> Self {
        Self {
            pool,
            blobs,
            registry,
            limits,
        }
    }

    /// Processes at most one durable import or one late operation correlation.
    ///
    /// The selector includes every non-terminal post-storage state, so a
    /// process crash never makes the item ineligible for startup recovery.
    ///
    /// # Errors
    ///
    /// Returns [`ReparseError`] when durable evidence, parsing, or persistence fails.
    pub async fn process_pending_once(&self) -> Result<usize, ReparseError> {
        let row: Option<(Uuid, Uuid, Uuid, Uuid, serde_json::Value, String, String)> =
            sqlx::query_as(
                "SELECT r.id, e.id, e.ai_archive_id, e.account_id, e.blob_ref,
                        e.acquisition_mode, r.state
                 FROM chatgpt_archive.import_runs r
                 JOIN chatgpt_archive.exports e ON e.id = r.export_id
                 WHERE r.state IN ('stored', 'inspected', 'parsed', 'reconciled')
                    OR (r.state IN ('completed', 'partial') AND EXISTS (
                        SELECT 1 FROM chatgpt_archive.platform_operation_imports p
                        WHERE p.import_run_id = r.id AND p.reported_at IS NULL))
                 ORDER BY EXISTS (
                     SELECT 1 FROM chatgpt_archive.platform_operation_imports p
                     WHERE p.import_run_id = r.id AND p.reported_at IS NULL
                 ) DESC, r.started_at, r.id
                 LIMIT 1",
            )
            .fetch_optional(&self.pool)
            .await?;
        let Some((run_id, export_id, archive_id, tenant_id, raw_json, mode, state)) = row else {
            return Ok(0);
        };

        if matches!(state.as_str(), "completed" | "partial") {
            let summary = load_import_summary(&self.pool, run_id).await?;
            enqueue_operation_reports(
                &self.pool, run_id, export_id, archive_id, tenant_id, summary,
            )
            .await?;
            return Ok(1);
        }

        let prepared = self
            .build_initial_plan(export_id, archive_id, tenant_id, raw_json, &mode)
            .await;
        let (plan, stored_artifacts) = match prepared {
            Ok(prepared) => prepared,
            Err(error) if is_permanent_import_error(&error) => {
                mark_permanent_failure(&self.pool, run_id, export_id, tenant_id).await?;
                return Ok(1);
            }
            Err(error) => return Err(error),
        };
        self.persist_initial_plan(run_id, archive_id, &plan, &stored_artifacts)
            .await?;
        Ok(1)
    }

    async fn build_initial_plan(
        &self,
        export_id: Uuid,
        archive_id: Uuid,
        tenant_id: Uuid,
        raw_json: serde_json::Value,
        mode: &str,
    ) -> Result<(ReparsePlan, Vec<BlobRef>), ReparseError> {
        let raw: BlobRef = serde_json::from_value(raw_json)?;
        self.blobs.verify(&raw).await?;
        let acquisition = AcquisitionMode::parse(mode).ok_or(ReparseError::Conflict)?;
        let inventory = ArchiveInspector::new(self.blobs.clone(), self.limits.clone())
            .inspect(&raw)
            .await?;
        let selected = match self.registry.select(&inventory, acquisition) {
            crate::ParserSelection::Selected(selected) => selected,
            crate::ParserSelection::Unsupported | crate::ParserSelection::Ambiguous(_) => {
                return Err(ReparseError::Conflict);
            }
        };
        let parser = self
            .registry
            .find_exact(&selected, &inventory, acquisition)
            .ok_or(ReparseError::Conflict)?;
        let artifacts = read_artifacts(&self.blobs, &raw, &inventory).await?;
        let parsed = parser.execute(ParserExecutionInput {
            inventory: &inventory,
            artifacts: &artifacts,
        })?;
        if parsed.parser != selected {
            return Err(ReparseError::Conflict);
        }

        let mut stored_artifacts = Vec::new();
        for artifact in &artifacts {
            stored_artifacts.push(
                self.blobs
                    .store(
                        "application/octet-stream",
                        stream::iter([Ok::<Bytes, std::io::Error>(artifact.bytes.clone())]),
                    )
                    .await?,
            );
        }
        let current = current_projection(&self.pool, export_id).await?;
        let report = compare(
            archive_id,
            selected,
            raw.digest.hex.as_str().to_owned(),
            &current,
            &parsed,
        );
        Ok((
            ReparsePlan {
                report,
                registry_fingerprint: String::new(),
                input_projection_fingerprint: String::new(),
                tenant_id,
                export_id,
                raw_ref: raw,
                inventory,
                artifacts,
                parsed,
                current,
            },
            stored_artifacts,
        ))
    }

    async fn persist_initial_plan(
        &self,
        run_id: Uuid,
        archive_id: Uuid,
        plan: &ReparsePlan,
        stored_artifacts: &[BlobRef],
    ) -> Result<(), ReparseError> {
        let summary = import_summary(&plan.parsed);
        let completeness_status = if summary.completeness
            == ratatoskr_ai_archive_contracts::AiArchiveCompleteness::Complete
        {
            "complete"
        } else {
            "structurally_partial"
        };
        let terminal_state = if summary.completeness
            == ratatoskr_ai_archive_contracts::AiArchiveCompleteness::Complete
        {
            "completed"
        } else {
            "partial"
        };

        let mut tx = self.pool.begin().await?;
        lock_tenant(&mut tx, plan.tenant_id).await?;
        let current_state: String = sqlx::query_scalar(
            "SELECT state FROM chatgpt_archive.import_runs WHERE id = $1 FOR UPDATE",
        )
        .bind(run_id)
        .fetch_one(&mut *tx)
        .await?;
        if matches!(current_state.as_str(), "completed" | "partial") {
            tx.rollback().await?;
            return Ok(());
        }
        persist_artifacts(&mut tx, plan, stored_artifacts).await?;
        persist_projection(&mut tx, plan).await?;
        persist_messages(&mut tx, plan).await?;
        let counts = serde_json::json!({
            "conversations": summary.conversation_count,
            "messages": summary.message_count,
            "assets": summary.asset_count,
            "gaps": summary.gap_count,
        });
        sqlx::query(
            "INSERT INTO chatgpt_archive.completeness_reports
             (id, import_run_id, status, counts, warnings, missing_assets, unknown_variants)
             VALUES ($1, $2, $3, $4, '[]'::jsonb, $5, $6)
             ON CONFLICT (import_run_id) DO NOTHING",
        )
        .bind(Uuid::now_v7())
        .bind(run_id)
        .bind(completeness_status)
        .bind(counts)
        .bind(
            i32::try_from(
                plan.parsed
                    .assets
                    .iter()
                    .filter(|asset| asset.blob.is_none())
                    .count(),
            )
            .unwrap_or(i32::MAX),
        )
        .bind(i32::try_from(plan.parsed.raw_records.len()).unwrap_or(i32::MAX))
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE chatgpt_archive.import_runs
             SET state = $2, parser_name = $3, parser_version = $4, schema_id = $5,
                 finished_at = now() WHERE id = $1",
        )
        .bind(run_id)
        .bind(terminal_state)
        .bind(&plan.parsed.parser.name)
        .bind(&plan.parsed.parser.version)
        .bind(&plan.parsed.schema_id)
        .execute(&mut *tx)
        .await?;
        enqueue_operation_reports_in(
            &mut tx,
            run_id,
            plan.export_id,
            archive_id,
            plan.tenant_id,
            summary,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

const fn is_permanent_import_error(error: &ReparseError) -> bool {
    matches!(
        error,
        ReparseError::Conflict
            | ReparseError::Intake(_)
            | ReparseError::Parser(_)
            | ReparseError::Encode(_)
            | ReparseError::Report(_)
            | ReparseError::Blob(
                crate::BlobStoreError::Missing | crate::BlobStoreError::InvalidMediaType
            )
    )
}

async fn mark_permanent_failure(
    pool: &PgPool,
    run_id: Uuid,
    export_id: Uuid,
    tenant_id: Uuid,
) -> Result<(), ReparseError> {
    let mut tx = pool.begin().await?;
    lock_tenant(&mut tx, tenant_id).await?;
    sqlx::query(
        "UPDATE chatgpt_archive.import_runs
         SET state = 'failed', finished_at = now()
         WHERE id = $1 AND state NOT IN ('completed', 'partial', 'failed')",
    )
    .bind(run_id)
    .execute(&mut *tx)
    .await?;
    let operations: Vec<Uuid> = sqlx::query_scalar(
        "SELECT operation_id FROM chatgpt_archive.platform_operation_imports
         WHERE import_run_id = $1 AND reported_at IS NULL ORDER BY operation_id",
    )
    .bind(run_id)
    .fetch_all(&mut *tx)
    .await?;
    for operation_id in operations {
        let report =
            crate::receipt::report::failed(crate::receipt::PlatformOperation { operation_id })?;
        let payload = operation_report_envelope(&report)?;
        sqlx::query(
            "INSERT INTO chatgpt_archive.outbox_events
             (event_type, aggregate_id, tenant_id, export_id, payload, correlation_id,
              deduplication_key)
             VALUES ('platform.operation.reported.v1', $1, $2, $3, $4, $1, $5)
             ON CONFLICT DO NOTHING",
        )
        .bind(operation_id)
        .bind(tenant_id)
        .bind(export_id)
        .bind(serde_json::to_value(payload)?)
        .bind(format!("platform.operation.reported.v1:{operation_id}"))
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE chatgpt_archive.platform_operation_imports SET reported_at = now()
             WHERE operation_id = $1 AND reported_at IS NULL",
        )
        .bind(operation_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

fn import_summary(parsed: &ParsedConversations) -> crate::receipt::report::ImportSummary {
    let conversations = parsed.conversations.len();
    let messages = parsed
        .conversations
        .iter()
        .map(|conversation| conversation.messages.len())
        .sum::<usize>();
    let stored_assets = parsed
        .assets
        .iter()
        .filter(|asset| asset.blob.is_some())
        .count();
    let unavailable_assets = parsed
        .assets
        .iter()
        .filter(|asset| asset.blob.is_none())
        .count();
    let unobserved_categories = usize::from(parsed.projects.is_empty())
        + usize::from(parsed.canvas_documents.is_empty())
        + usize::from(parsed.assets.is_empty());
    let gaps = parsed.raw_records.len() + unavailable_assets + unobserved_categories;
    let completeness = if gaps == 0 {
        ratatoskr_ai_archive_contracts::AiArchiveCompleteness::Complete
    } else {
        ratatoskr_ai_archive_contracts::AiArchiveCompleteness::StructurallyPartial
    };
    crate::receipt::report::ImportSummary {
        completeness,
        conversation_count: u32::try_from(conversations).unwrap_or(u32::MAX),
        message_count: u32::try_from(messages).unwrap_or(u32::MAX),
        asset_count: u32::try_from(stored_assets).unwrap_or(u32::MAX),
        gap_count: u32::try_from(gaps).unwrap_or(u32::MAX),
        warning_count: 0,
    }
}

async fn load_import_summary(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<crate::receipt::report::ImportSummary, ReparseError> {
    let (status, counts, warnings): (String, serde_json::Value, serde_json::Value) =
        sqlx::query_as(
            "SELECT status, counts, warnings FROM chatgpt_archive.completeness_reports
         WHERE import_run_id = $1",
        )
        .bind(run_id)
        .fetch_one(pool)
        .await?;
    let count = |name: &str| {
        counts
            .get(name)
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or_default()
    };
    let completeness = if status == "complete" {
        ratatoskr_ai_archive_contracts::AiArchiveCompleteness::Complete
    } else {
        ratatoskr_ai_archive_contracts::AiArchiveCompleteness::StructurallyPartial
    };
    Ok(crate::receipt::report::ImportSummary {
        completeness,
        conversation_count: count("conversations"),
        message_count: count("messages"),
        asset_count: count("assets"),
        gap_count: count("gaps"),
        warning_count: warnings
            .as_array()
            .map_or(0, |values| u32::try_from(values.len()).unwrap_or(u32::MAX)),
    })
}

async fn enqueue_operation_reports(
    pool: &PgPool,
    run_id: Uuid,
    export_id: Uuid,
    archive_id: Uuid,
    tenant_id: Uuid,
    summary: crate::receipt::report::ImportSummary,
) -> Result<(), ReparseError> {
    let mut tx = pool.begin().await?;
    enqueue_operation_reports_in(&mut tx, run_id, export_id, archive_id, tenant_id, summary)
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn enqueue_operation_reports_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    run_id: Uuid,
    export_id: Uuid,
    archive_id: Uuid,
    tenant_id: Uuid,
    summary: crate::receipt::report::ImportSummary,
) -> Result<(), ReparseError> {
    let operations: Vec<Uuid> = sqlx::query_scalar(
        "SELECT operation_id FROM chatgpt_archive.platform_operation_imports
         WHERE import_run_id = $1 AND reported_at IS NULL ORDER BY operation_id",
    )
    .bind(run_id)
    .fetch_all(&mut **tx)
    .await?;
    for operation_id in operations {
        let report = crate::receipt::report::imported(
            crate::receipt::PlatformOperation { operation_id },
            archive_id,
            summary,
        )?;
        let payload = operation_report_envelope(&report)?;
        sqlx::query(
            "INSERT INTO chatgpt_archive.outbox_events
             (event_type, aggregate_id, tenant_id, export_id, payload, correlation_id,
              deduplication_key)
             VALUES ('platform.operation.reported.v1', $1, $2, $3, $4, $1, $5)
             ON CONFLICT DO NOTHING",
        )
        .bind(operation_id)
        .bind(tenant_id)
        .bind(export_id)
        .bind(serde_json::to_value(payload)?)
        .bind(format!("platform.operation.reported.v1:{operation_id}"))
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "UPDATE chatgpt_archive.platform_operation_imports SET reported_at = now()
             WHERE operation_id = $1 AND reported_at IS NULL",
        )
        .bind(operation_id)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

fn operation_report_envelope(
    report: &ratatoskr_operation_contracts::OperationReported,
) -> Result<EventEnvelope, ReparseError> {
    let event_id = EventId::new_v7();
    let mut envelope: EventEnvelope = serde_json::from_value(serde_json::json!({
        "event_id": event_id,
        "event_type": ratatoskr_operation_contracts::OperationReported::EVENT_TYPE,
        "occurred_at": WireTimestamp::now(),
        "producer": "ratatoskr-chatgpt",
        "aggregate_id": EntityRef::from(report.operation_id),
        "correlation_id": event_id.as_entity_ref(),
        "schema_version": 1,
        "payload": {}
    }))?;
    envelope
        .set_payload(report)
        .map_err(|_| ReparseError::Conflict)?;
    Ok(envelope)
}

async fn persist_messages(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    plan: &ReparsePlan,
) -> Result<(), ReparseError> {
    for conversation in &plan.parsed.conversations {
        let conversation_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM chatgpt_archive.conversations
             WHERE account_id = $1 AND external_id = $2 ORDER BY id LIMIT 1",
        )
        .bind(plan.tenant_id)
        .bind(&conversation.external_id)
        .fetch_one(&mut **tx)
        .await?;
        let mut message_ids = BTreeMap::new();
        for message in &conversation.messages {
            let message_id: Uuid = sqlx::query_scalar(
                "INSERT INTO chatgpt_archive.messages
                 (id, conversation_id, external_id, role, model_slug, provider_metadata)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (conversation_id, external_id) DO UPDATE
                 SET role = EXCLUDED.role, model_slug = EXCLUDED.model_slug,
                     provider_metadata = EXCLUDED.provider_metadata
                 RETURNING id",
            )
            .bind(Uuid::now_v7())
            .bind(conversation_id)
            .bind(&message.external_id)
            .bind(message_role(&message.role))
            .bind(&message.model_slug)
            .bind(&message.provider_metadata)
            .fetch_one(&mut **tx)
            .await?;
            message_ids.insert(message.external_id.clone(), message_id);
            sqlx::query(
                "INSERT INTO chatgpt_archive.export_entity_observations
                 (export_id, entity_kind, entity_id) VALUES ($1, 'message', $2)
                 ON CONFLICT DO NOTHING",
            )
            .bind(plan.export_id)
            .bind(message_id)
            .execute(&mut **tx)
            .await?;
            for part in &message.parts {
                sqlx::query(
                    "INSERT INTO chatgpt_archive.content_parts
                     (id, message_id, revision, ordinal, part_kind, payload)
                     VALUES ($1, $2, 0, $3, $4, $5) ON CONFLICT DO NOTHING",
                )
                .bind(Uuid::now_v7())
                .bind(message_id)
                .bind(i32::try_from(part.ordinal).unwrap_or(i32::MAX))
                .bind(content_kind(&part.kind))
                .bind(&part.payload)
                .execute(&mut **tx)
                .await?;
            }
        }
        for message in &conversation.messages {
            if let (Some(message_id), Some(parent_id)) = (
                message_ids.get(&message.external_id),
                message
                    .parent_external_id
                    .as_ref()
                    .and_then(|parent| message_ids.get(parent)),
            ) {
                sqlx::query(
                    "UPDATE chatgpt_archive.messages SET parent_message_id = $2 WHERE id = $1",
                )
                .bind(message_id)
                .bind(parent_id)
                .execute(&mut **tx)
                .await?;
            }
        }
    }
    Ok(())
}

const fn message_role(role: &crate::MessageRole) -> &'static str {
    match role {
        crate::MessageRole::System => "system",
        crate::MessageRole::User => "user",
        crate::MessageRole::Assistant => "assistant",
        crate::MessageRole::Tool => "tool",
        crate::MessageRole::Internal => "internal",
        crate::MessageRole::Unknown => "unknown",
    }
}

const fn content_kind(kind: &crate::ContentPartKind) -> &'static str {
    match kind {
        crate::ContentPartKind::Text => "text",
        crate::ContentPartKind::ToolCall => "tool_call",
        crate::ContentPartKind::ToolResult => "tool_result",
        crate::ContentPartKind::Image => "image",
        crate::ContentPartKind::File => "file",
        crate::ContentPartKind::Unknown => "unknown",
    }
}
