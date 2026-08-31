//! Verified, deterministic replay of preserved raw archives.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::stream;
use ratatoskr_identifiers::BlobRef;
use serde::{Deserialize, Serialize};
use sha2::Digest as _;
use sqlx::PgPool;
use uuid::Uuid;

use crate::privacy_deletion::service::lock_tenant;
use crate::reconciliation::digest::conversation_digest;
use crate::{
    AcquisitionMode, ArchiveInspector, ArchiveInventory, ArchiveLimits, BlobStore, EntryKind,
    ParsedConversations, ParserArtifactEvidence, ParserExecutionError, ParserExecutionInput,
    ParserId, ParserRegistry,
};

/// Classification of one normalized subject comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReparseChangeKind {
    /// Newly evidenced subject.
    Added,
    /// Existing subject with a different normalized digest.
    Changed,
    /// Existing subject with an identical normalized digest.
    Unchanged,
    /// Existing subject omitted by the newer parser and retained.
    ProposedRemoval,
}

/// One deterministic subject-level comparison entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReparseChange {
    /// Normalized subject class.
    pub subject_kind: String,
    /// Stable provider identity within this private operator report.
    pub subject_id: String,
    /// Comparison result.
    pub kind: ReparseChangeKind,
}

/// Content-free coverage or validation warning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReparseWarning {
    /// Stable machine-readable warning code.
    pub code: String,
    /// Related subject identity when available.
    pub subject_id: Option<String>,
}

/// Stable JSON report shared by dry-run and immediate apply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReparseReport {
    /// Selected archive identity.
    pub archive_id: Uuid,
    /// Exact target parser.
    pub target_parser: ParserId,
    /// Immutable raw archive digest.
    pub raw_digest: String,
    /// Sorted subject comparisons.
    pub changes: Vec<ReparseChange>,
    /// Sorted content-free warnings.
    pub warnings: Vec<ReparseWarning>,
    /// Sorted downstream event subjects proposed by the comparison.
    pub event_subjects: Vec<String>,
    /// Conservative completeness class.
    pub completeness: String,
}

/// Immutable comparison bound to all inputs required for safe apply.
#[derive(Debug, Clone)]
pub struct ReparsePlan {
    /// Operator-visible deterministic report.
    pub report: ReparseReport,
    /// Parser registry fingerprint.
    pub registry_fingerprint: String,
    /// Current normalized projection fingerprint.
    pub input_projection_fingerprint: String,
    pub(crate) tenant_id: Uuid,
    pub(crate) export_id: Uuid,
    pub(crate) raw_ref: BlobRef,
    pub(crate) inventory: ArchiveInventory,
    pub(crate) artifacts: Vec<ParserArtifactEvidence>,
    pub(crate) parsed: ParsedConversations,
    pub(crate) current: BTreeMap<String, CurrentConversation>,
}

#[derive(Debug, Clone)]
pub(crate) struct CurrentConversation {
    id: Uuid,
    digest: String,
}

/// Reparse planning or application failure without archive content.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReparseError {
    /// The selected input or parser is unknown, incompatible, stale, or blocked.
    #[error("reparse input is unavailable or incompatible")]
    Conflict,
    /// Owned persistence failed.
    #[error("reparse persistence failed")]
    Store(#[from] sqlx::Error),
    /// Raw evidence failed verification or hostile reinspection.
    #[error("reparse raw evidence failed inspection")]
    Intake(#[from] crate::ArchiveIntakeError),
    /// The exact compiled parser failed without exposing content.
    #[error("reparse parser execution failed")]
    Parser(#[from] ParserExecutionError),
    /// A durable report or reference could not be encoded.
    #[error("reparse evidence encoding failed")]
    Encode(#[from] serde_json::Error),
    /// Extracted evidence could not be stored for apply.
    #[error("reparse artifact storage failed")]
    Blob(#[from] crate::BlobStoreError),
    /// A terminal Platform operation result could not be constructed.
    #[error("import result construction failed")]
    Report(#[from] crate::receipt::RepositoryError),
}

/// Plans and applies exact parser replay over one retained raw archive.
#[derive(Debug, Clone)]
pub struct ReparseEngine {
    pool: PgPool,
    blobs: BlobStore,
    registry: Arc<ParserRegistry>,
    limits: ArchiveLimits,
}

impl ReparseEngine {
    /// Creates an engine from process-owned storage and compiled parsers.
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

    /// Builds a side-effect-free immutable comparison plan.
    ///
    /// # Errors
    ///
    /// Returns [`ReparseError`] when evidence or exact parser selection fails.
    pub async fn plan(
        &self,
        tenant_id: Uuid,
        archive_id: Uuid,
        target_parser: ParserId,
    ) -> Result<ReparsePlan, ReparseError> {
        if privacy_blocked(&self.pool, tenant_id).await? {
            return Err(ReparseError::Conflict);
        }
        let row: Option<(Uuid, serde_json::Value, String, String)> = sqlx::query_as(
            "SELECT e.id, e.blob_ref, e.acquisition_mode,
                    COALESCE((SELECT versions.parser_name || '@' || versions.parser_version
                              FROM (
                                SELECT r.parser_name, r.parser_version, r.started_at AS observed_at
                                FROM chatgpt_archive.import_runs r
                                WHERE r.export_id = e.id AND r.parser_name IS NOT NULL
                                      AND r.parser_version IS NOT NULL
                                UNION ALL
                                SELECT p.parser_name, p.parser_version,
                                       COALESCE(p.completed_at, p.started_at) AS observed_at
                                FROM chatgpt_archive.reparse_runs p
                                WHERE p.export_id = e.id AND p.status IN ('applied', 'unchanged')
                              ) versions
                              ORDER BY versions.observed_at DESC LIMIT 1), '')
             FROM chatgpt_archive.exports e
             WHERE e.account_id = $1 AND e.ai_archive_id = $2",
        )
        .bind(tenant_id)
        .bind(archive_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some((export_id, raw_json, mode, current_parser)) = row else {
            return Err(ReparseError::Conflict);
        };
        let raw_ref: BlobRef = serde_json::from_value(raw_json)?;
        self.blobs.verify(&raw_ref).await?;
        let acquisition_mode = AcquisitionMode::parse(&mode).ok_or(ReparseError::Conflict)?;
        let inventory = ArchiveInspector::new(self.blobs.clone(), self.limits.clone())
            .inspect(&raw_ref)
            .await?;
        let compatible = self
            .registry
            .compatible_versions(&inventory, acquisition_mode);
        let current_index = compatible
            .iter()
            .position(|parser| format!("{}@{}", parser.name, parser.version) == current_parser)
            .ok_or(ReparseError::Conflict)?;
        let target_index = compatible
            .iter()
            .position(|parser| parser == &target_parser)
            .ok_or(ReparseError::Conflict)?;
        if target_index <= current_index {
            return Err(ReparseError::Conflict);
        }
        let compiled = self
            .registry
            .find_exact(&target_parser, &inventory, acquisition_mode)
            .ok_or(ReparseError::Conflict)?;
        let artifacts = read_artifacts(&self.blobs, &raw_ref, &inventory).await?;
        let parsed = compiled.execute(ParserExecutionInput {
            inventory: &inventory,
            artifacts: &artifacts,
        })?;
        if parsed.parser != target_parser {
            return Err(ReparseError::Conflict);
        }
        let current = current_projection(&self.pool, export_id).await?;
        let input_projection_fingerprint = projection_fingerprint(&current);
        let registry_fingerprint = parser_fingerprint(&compatible);
        let report = compare(
            archive_id,
            target_parser,
            raw_ref.digest.hex.as_str().to_owned(),
            &current,
            &parsed,
        );
        Ok(ReparsePlan {
            report,
            registry_fingerprint,
            input_projection_fingerprint,
            tenant_id,
            export_id,
            raw_ref,
            inventory,
            artifacts,
            parsed,
            current,
        })
    }

    /// Applies an unchanged immutable plan and returns its report.
    ///
    /// # Errors
    ///
    /// Returns [`ReparseError`] when fingerprints are stale or persistence fails.
    pub async fn apply(&self, plan: &ReparsePlan) -> Result<ReparseReport, ReparseError> {
        if privacy_blocked(&self.pool, plan.tenant_id).await? {
            return Err(ReparseError::Conflict);
        }
        if let Some(report) = prior_report(&self.pool, plan).await? {
            return Ok(report);
        }
        let current = current_projection(&self.pool, plan.export_id).await?;
        if projection_fingerprint(&current) != plan.input_projection_fingerprint {
            return Err(ReparseError::Conflict);
        }
        let compatible = self.registry.compatible_versions(
            &plan.inventory,
            acquisition_mode_for_export(&self.pool, plan.export_id).await?,
        );
        if parser_fingerprint(&compatible) != plan.registry_fingerprint {
            return Err(ReparseError::Conflict);
        }
        let raw_json: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT blob_ref FROM chatgpt_archive.exports WHERE id = $1 AND account_id = $2",
        )
        .bind(plan.export_id)
        .bind(plan.tenant_id)
        .fetch_optional(&self.pool)
        .await?;
        let raw: BlobRef = serde_json::from_value(raw_json.ok_or(ReparseError::Conflict)?)?;
        if raw.digest != plan.raw_ref.digest {
            return Err(ReparseError::Conflict);
        }
        self.blobs.verify(&raw).await?;

        let mut stored_artifacts = Vec::new();
        for artifact in &plan.artifacts {
            let reference = self
                .blobs
                .store(
                    "application/octet-stream",
                    stream::iter([Ok::<Bytes, std::io::Error>(artifact.bytes.clone())]),
                )
                .await?;
            stored_artifacts.push(reference);
        }

        let mut transaction = self.pool.begin().await?;
        lock_tenant(&mut transaction, plan.tenant_id).await?;
        if let Some(report) = prior_report_in_transaction(&mut transaction, plan).await? {
            transaction.rollback().await?;
            return Ok(report);
        }
        let run_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO chatgpt_archive.reparse_runs
             (id, tenant_id, export_id, parser_name, parser_version, raw_sha256_hex,
              registry_fingerprint, input_projection_fingerprint, status, report, completed_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'applied', $9, now())",
        )
        .bind(run_id)
        .bind(plan.tenant_id)
        .bind(plan.export_id)
        .bind(&plan.report.target_parser.name)
        .bind(&plan.report.target_parser.version)
        .bind(&plan.report.raw_digest)
        .bind(&plan.registry_fingerprint)
        .bind(&plan.input_projection_fingerprint)
        .bind(serde_json::to_value(&plan.report)?)
        .execute(&mut *transaction)
        .await?;
        persist_artifacts(&mut transaction, plan, &stored_artifacts).await?;
        persist_projection(&mut transaction, plan).await?;
        transaction.commit().await?;
        Ok(plan.report.clone())
    }

    pub(crate) fn pool(&self) -> &PgPool {
        &self.pool
    }
}

async fn prior_report(
    pool: &PgPool,
    plan: &ReparsePlan,
) -> Result<Option<ReparseReport>, ReparseError> {
    let value: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT report FROM chatgpt_archive.reparse_runs
         WHERE tenant_id = $1 AND export_id = $2 AND parser_name = $3 AND parser_version = $4
           AND raw_sha256_hex = $5 AND registry_fingerprint = $6
           AND input_projection_fingerprint = $7",
    )
    .bind(plan.tenant_id)
    .bind(plan.export_id)
    .bind(&plan.report.target_parser.name)
    .bind(&plan.report.target_parser.version)
    .bind(&plan.report.raw_digest)
    .bind(&plan.registry_fingerprint)
    .bind(&plan.input_projection_fingerprint)
    .fetch_optional(pool)
    .await?;
    value
        .map(serde_json::from_value)
        .transpose()
        .map_err(Into::into)
}

async fn prior_report_in_transaction(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    plan: &ReparsePlan,
) -> Result<Option<ReparseReport>, ReparseError> {
    let value: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT report FROM chatgpt_archive.reparse_runs
         WHERE tenant_id = $1 AND export_id = $2 AND parser_name = $3 AND parser_version = $4
           AND raw_sha256_hex = $5 AND registry_fingerprint = $6
           AND input_projection_fingerprint = $7",
    )
    .bind(plan.tenant_id)
    .bind(plan.export_id)
    .bind(&plan.report.target_parser.name)
    .bind(&plan.report.target_parser.version)
    .bind(&plan.report.raw_digest)
    .bind(&plan.registry_fingerprint)
    .bind(&plan.input_projection_fingerprint)
    .fetch_optional(&mut **transaction)
    .await?;
    value
        .map(serde_json::from_value)
        .transpose()
        .map_err(Into::into)
}

async fn privacy_blocked(pool: &PgPool, tenant_id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM chatgpt_archive.privacy_deletion_requests
         WHERE tenant_id = $1 AND status <> 'completed')",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await
}

async fn acquisition_mode_for_export(
    pool: &PgPool,
    export_id: Uuid,
) -> Result<AcquisitionMode, ReparseError> {
    let mode: String =
        sqlx::query_scalar("SELECT acquisition_mode FROM chatgpt_archive.exports WHERE id = $1")
            .bind(export_id)
            .fetch_one(pool)
            .await?;
    AcquisitionMode::parse(&mode).ok_or(ReparseError::Conflict)
}

pub(crate) async fn read_artifacts(
    blobs: &BlobStore,
    raw: &BlobRef,
    inventory: &ArchiveInventory,
) -> Result<Vec<ParserArtifactEvidence>, ReparseError> {
    let path = blobs.verify(raw).await?;
    let inventory = inventory.clone();
    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(path).map_err(|_| ReparseError::Conflict)?;
        let mut archive = zip::ZipArchive::new(file).map_err(|_| ReparseError::Conflict)?;
        let mut artifacts = Vec::new();
        for (index, entry) in inventory.entries.iter().enumerate() {
            if entry.kind == EntryKind::Directory {
                continue;
            }
            let mut zipped = archive
                .by_index(index)
                .map_err(|_| ReparseError::Conflict)?;
            let capacity =
                usize::try_from(entry.decompressed_bytes).map_err(|_| ReparseError::Conflict)?;
            let mut bytes = Vec::with_capacity(capacity);
            zipped
                .read_to_end(&mut bytes)
                .map_err(|_| ReparseError::Conflict)?;
            if bytes.len() != capacity {
                return Err(ReparseError::Conflict);
            }
            artifacts.push(ParserArtifactEvidence {
                path: entry.path.clone(),
                bytes: Bytes::from(bytes),
                quarantined: matches!(entry.kind, EntryKind::Html | EntryKind::Media),
            });
        }
        Ok(artifacts)
    })
    .await
    .map_err(|_| ReparseError::Conflict)?
}

pub(crate) async fn current_projection(
    pool: &PgPool,
    export_id: Uuid,
) -> Result<BTreeMap<String, CurrentConversation>, sqlx::Error> {
    let rows: Vec<(Uuid, String, Option<String>)> = sqlx::query_as(
        "SELECT c.id, c.external_id,
           (SELECT r.payload ->> 'digest' FROM chatgpt_archive.revisions r
            WHERE r.entity_table = 'conversations' AND r.entity_id = c.id
            ORDER BY r.revision_number DESC LIMIT 1)
         FROM chatgpt_archive.conversations c
         JOIN chatgpt_archive.export_entity_observations o
           ON o.entity_kind = 'conversation' AND o.entity_id = c.id
         WHERE o.export_id = $1 ORDER BY c.external_id",
    )
    .bind(export_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, external_id, digest)| {
            (
                external_id,
                CurrentConversation {
                    id,
                    digest: digest.unwrap_or_default(),
                },
            )
        })
        .collect())
}

pub(crate) fn compare(
    archive_id: Uuid,
    target_parser: ParserId,
    raw_digest: String,
    current: &BTreeMap<String, CurrentConversation>,
    parsed: &ParsedConversations,
) -> ReparseReport {
    let parsed_digests: BTreeMap<_, _> = parsed
        .conversations
        .iter()
        .map(|conversation| {
            (
                conversation.external_id.clone(),
                conversation_digest(conversation),
            )
        })
        .collect();
    let mut changes = Vec::new();
    let mut warnings = Vec::new();
    let mut event_subjects = Vec::new();
    for (id, digest) in &parsed_digests {
        let kind = match current.get(id) {
            None => ReparseChangeKind::Added,
            Some(existing) if existing.digest == *digest => ReparseChangeKind::Unchanged,
            Some(_) => ReparseChangeKind::Changed,
        };
        if kind == ReparseChangeKind::Added {
            event_subjects.push("ai_archive.conversation.added.v1".to_owned());
        } else if kind == ReparseChangeKind::Changed {
            event_subjects.push("ai_archive.conversation.updated.v1".to_owned());
        }
        changes.push(ReparseChange {
            subject_kind: "conversation".to_owned(),
            subject_id: id.clone(),
            kind,
        });
    }
    for id in current
        .keys()
        .filter(|id| !parsed_digests.contains_key(*id))
    {
        changes.push(ReparseChange {
            subject_kind: "conversation".to_owned(),
            subject_id: id.clone(),
            kind: ReparseChangeKind::ProposedRemoval,
        });
        warnings.push(ReparseWarning {
            code: "coverage_omission".to_owned(),
            subject_id: Some(id.clone()),
        });
    }
    changes.sort_by(|left, right| {
        (&left.subject_kind, &left.subject_id).cmp(&(&right.subject_kind, &right.subject_id))
    });
    warnings.sort_by(|left, right| {
        (&left.code, &left.subject_id).cmp(&(&right.code, &right.subject_id))
    });
    event_subjects.sort();
    ReparseReport {
        archive_id,
        target_parser,
        raw_digest,
        changes,
        warnings,
        event_subjects,
        completeness: "structurally_partial".to_owned(),
    }
}

fn projection_fingerprint(current: &BTreeMap<String, CurrentConversation>) -> String {
    let values: Vec<_> = current
        .iter()
        .map(|(id, conversation)| (id, &conversation.digest))
        .collect();
    hex::encode(sha2::Sha256::digest(
        serde_json::to_vec(&values).unwrap_or_default(),
    ))
}

fn parser_fingerprint(parsers: &[ParserId]) -> String {
    hex::encode(sha2::Sha256::digest(
        serde_json::to_vec(parsers).unwrap_or_default(),
    ))
}

pub(crate) async fn persist_artifacts(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    plan: &ReparsePlan,
    references: &[BlobRef],
) -> Result<(), ReparseError> {
    for (ordinal, reference) in references.iter().enumerate() {
        let ordinal = i32::try_from(ordinal).map_err(|_| ReparseError::Conflict)?;
        sqlx::query(
            "INSERT INTO chatgpt_archive.extracted_artifacts
             (id, export_id, artifact_ordinal, artifact_kind, blob_ref, sha256_hex, byte_length)
             VALUES ($1, $2, $3, 'entry', $4, $5, $6)
             ON CONFLICT (export_id, artifact_ordinal) DO NOTHING",
        )
        .bind(Uuid::now_v7())
        .bind(plan.export_id)
        .bind(ordinal)
        .bind(serde_json::to_value(reference)?)
        .bind(reference.digest.hex.as_str())
        .bind(i64::try_from(reference.length_bytes).map_err(|_| ReparseError::Conflict)?)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

pub(crate) async fn persist_projection(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    plan: &ReparsePlan,
) -> Result<(), ReparseError> {
    for parsed in &plan.parsed.conversations {
        let digest = conversation_digest(parsed);
        let id = plan
            .current
            .get(&parsed.external_id)
            .map_or_else(Uuid::now_v7, |current| current.id);
        sqlx::query(
            "INSERT INTO chatgpt_archive.conversations (id, account_id, external_id)
             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .bind(plan.tenant_id)
        .bind(&parsed.external_id)
        .execute(&mut **transaction)
        .await?;
        sqlx::query(
            "INSERT INTO chatgpt_archive.export_entity_observations
             (export_id, entity_kind, entity_id) VALUES ($1, 'conversation', $2)
             ON CONFLICT DO NOTHING",
        )
        .bind(plan.export_id)
        .bind(id)
        .execute(&mut **transaction)
        .await?;
        let unchanged = plan
            .current
            .get(&parsed.external_id)
            .is_some_and(|current| current.digest == digest);
        if unchanged {
            continue;
        }
        let revision: i32 = sqlx::query_scalar(
            "SELECT COALESCE(max(revision_number), 0) + 1
             FROM chatgpt_archive.revisions
             WHERE entity_table = 'conversations' AND entity_id = $1",
        )
        .bind(id)
        .fetch_one(&mut **transaction)
        .await?;
        sqlx::query(
            "INSERT INTO chatgpt_archive.revisions
             (id, entity_table, entity_id, revision_number, observed_in, payload)
             VALUES ($1, 'conversations', $2, $3, $4, $5)",
        )
        .bind(Uuid::now_v7())
        .bind(id)
        .bind(revision)
        .bind(plan.export_id)
        .bind(serde_json::json!({
            "digest": digest,
            "parser_name": plan.report.target_parser.name,
            "parser_version": plan.report.target_parser.version,
        }))
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}
