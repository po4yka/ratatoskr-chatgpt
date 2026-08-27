//! Deterministic reports for advancing retained archives to one exact parser.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ParserId;
use crate::reparse::{ReparseChangeKind, ReparseEngine, ReparseError, ReparsePlan};

/// Classification of one archive in a migration plan or result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParserMigrationEntryStatus {
    /// Reparse can be planned for this archive.
    Eligible,
    /// The archive already uses the requested parser.
    AlreadyCurrent,
    /// The requested parser is not compatible with this archive.
    Unsupported,
    /// Preserved raw evidence cannot be verified.
    RawMissing,
    /// An overlapping privacy operation blocks reprocessing.
    PrivacyBlocked,
    /// Hostile inspection failed.
    FailedInspection,
    /// Reparse applied changed normalized evidence.
    Applied,
    /// Reparse applied but normalized evidence was identical.
    Unchanged,
    /// An eligible archive failed during apply.
    Failed,
}

impl ParserMigrationEntryStatus {
    fn key(self) -> &'static str {
        match self {
            Self::Eligible => "eligible",
            Self::AlreadyCurrent => "already_current",
            Self::Unsupported => "unsupported",
            Self::RawMissing => "raw_missing",
            Self::PrivacyBlocked => "privacy_blocked",
            Self::FailedInspection => "failed_inspection",
            Self::Applied => "applied",
            Self::Unchanged => "unchanged",
            Self::Failed => "failed",
        }
    }
}

/// One content-free archive result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParserMigrationEntry {
    /// Stable archive identity.
    pub archive_id: Uuid,
    /// Exactly one classification.
    pub status: ParserMigrationEntryStatus,
}

/// Terminal state of a migration report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParserMigrationStatus {
    /// Side-effect-free planning result.
    Planned,
    /// All eligible archives finished without an apply failure.
    Completed,
    /// At least one eligible archive failed while another result was retained.
    Partial,
}

/// Stable tenant-scoped migration report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParserMigrationReport {
    /// Stable operator operation identity.
    pub operation_id: Uuid,
    /// Authenticated tenant identity.
    pub tenant_id: Uuid,
    /// Exact target parser.
    pub target_parser: ParserId,
    /// Planning or terminal execution state.
    pub status: ParserMigrationStatus,
    /// Entries in stable archive identity order.
    pub entries: Vec<ParserMigrationEntry>,
    /// Counts derived from the entries by status name.
    pub totals: BTreeMap<String, usize>,
}

impl ParserMigrationReport {
    /// Builds the initial plan report.
    #[must_use]
    pub fn planned(
        operation_id: Uuid,
        tenant_id: Uuid,
        target_parser: ParserId,
        mut entries: Vec<ParserMigrationEntry>,
    ) -> Self {
        entries.sort_by_key(|entry| entry.archive_id);
        let totals = totals(&entries);
        Self {
            operation_id,
            tenant_id,
            target_parser,
            status: ParserMigrationStatus::Planned,
            entries,
            totals,
        }
    }

    fn finished(
        operation_id: Uuid,
        tenant_id: Uuid,
        target_parser: ParserId,
        mut entries: Vec<ParserMigrationEntry>,
    ) -> Self {
        entries.sort_by_key(|entry| entry.archive_id);
        let status = if entries
            .iter()
            .any(|entry| entry.status == ParserMigrationEntryStatus::Failed)
        {
            ParserMigrationStatus::Partial
        } else {
            ParserMigrationStatus::Completed
        };
        let totals = totals(&entries);
        Self {
            operation_id,
            tenant_id,
            target_parser,
            status,
            entries,
            totals,
        }
    }
}

fn totals(entries: &[ParserMigrationEntry]) -> BTreeMap<String, usize> {
    let mut totals = BTreeMap::new();
    for entry in entries {
        *totals.entry(entry.status.key().to_owned()).or_insert(0) += 1;
    }
    totals
}

/// Immutable tenant migration plan. Planning performs no writes.
#[derive(Debug, Clone)]
pub struct ParserMigrationPlan {
    /// Deterministic operator-visible plan report.
    pub report: ParserMigrationReport,
    eligible: BTreeMap<Uuid, ReparsePlan>,
}

/// Migration planning or report persistence failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ParserMigrationError {
    /// Tenant export enumeration or report persistence failed.
    #[error("parser migration persistence failed")]
    Store(#[from] sqlx::Error),
    /// A stable durable report could not be encoded.
    #[error("parser migration report encoding failed")]
    Encode(#[from] serde_json::Error),
}

/// Tenant-scoped orchestration over the exact reparse engine.
#[derive(Debug, Clone)]
pub struct ParserMigrationEngine {
    reparse: ReparseEngine,
}

impl ParserMigrationEngine {
    /// Creates migration orchestration over one process-owned reparse engine.
    #[must_use]
    pub fn new(reparse: ReparseEngine) -> Self {
        Self { reparse }
    }

    /// Classifies every retained archive for one tenant without persistence.
    ///
    /// # Errors
    ///
    /// Returns [`ParserMigrationError`] when tenant archive enumeration fails.
    pub async fn plan(
        &self,
        operation_id: Uuid,
        tenant_id: Uuid,
        target_parser: ParserId,
    ) -> Result<ParserMigrationPlan, ParserMigrationError> {
        let privacy_blocked: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM chatgpt_archive.privacy_deletion_requests
             WHERE tenant_id = $1 AND status <> 'completed')",
        )
        .bind(tenant_id)
        .fetch_one(self.reparse.pool())
        .await?;
        let rows: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT e.ai_archive_id,
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
             WHERE e.account_id = $1 ORDER BY e.ai_archive_id",
        )
        .bind(tenant_id)
        .fetch_all(self.reparse.pool())
        .await?;
        let exact_target = format!("{}@{}", target_parser.name, target_parser.version);
        let mut entries = Vec::with_capacity(rows.len());
        let mut eligible = BTreeMap::new();
        for (archive_id, current_parser) in rows {
            let status = if privacy_blocked {
                ParserMigrationEntryStatus::PrivacyBlocked
            } else if current_parser == exact_target {
                ParserMigrationEntryStatus::AlreadyCurrent
            } else {
                match self
                    .reparse
                    .plan(tenant_id, archive_id, target_parser.clone())
                    .await
                {
                    Ok(plan) => {
                        eligible.insert(archive_id, plan);
                        ParserMigrationEntryStatus::Eligible
                    }
                    Err(ReparseError::Blob(_)) => ParserMigrationEntryStatus::RawMissing,
                    Err(ReparseError::Intake(_) | ReparseError::Parser(_)) => {
                        ParserMigrationEntryStatus::FailedInspection
                    }
                    Err(_) => ParserMigrationEntryStatus::Unsupported,
                }
            };
            entries.push(ParserMigrationEntry { archive_id, status });
        }
        Ok(ParserMigrationPlan {
            report: ParserMigrationReport::planned(operation_id, tenant_id, target_parser, entries),
            eligible,
        })
    }

    /// Applies eligible archives independently and persists one stable report.
    ///
    /// # Errors
    ///
    /// Returns [`ParserMigrationError`] only when the batch report cannot be
    /// persisted; archive-local reparse failures are retained in the report.
    pub async fn apply(
        &self,
        plan: &ParserMigrationPlan,
    ) -> Result<ParserMigrationReport, ParserMigrationError> {
        if let Some(report) = self
            .prior_report(plan.report.tenant_id, plan.report.operation_id)
            .await?
        {
            return Ok(report);
        }
        let mut entries = plan.report.entries.clone();
        for entry in &mut entries {
            if entry.status != ParserMigrationEntryStatus::Eligible {
                continue;
            }
            let Some(reparse_plan) = plan.eligible.get(&entry.archive_id) else {
                entry.status = ParserMigrationEntryStatus::Failed;
                continue;
            };
            entry.status = match self.reparse.apply(reparse_plan).await {
                Ok(report)
                    if report.changes.iter().all(|change| {
                        matches!(
                            change.kind,
                            ReparseChangeKind::Unchanged | ReparseChangeKind::ProposedRemoval
                        )
                    }) =>
                {
                    ParserMigrationEntryStatus::Unchanged
                }
                Ok(_) => ParserMigrationEntryStatus::Applied,
                Err(_) => ParserMigrationEntryStatus::Failed,
            };
        }
        let report = ParserMigrationReport::finished(
            plan.report.operation_id,
            plan.report.tenant_id,
            plan.report.target_parser.clone(),
            entries,
        );
        sqlx::query(
            "INSERT INTO chatgpt_archive.parser_migration_reports
             (id, tenant_id, operation_key, parser_name, parser_version, dry_run, status, report,
              completed_at)
             VALUES ($1, $2, $3, $4, $5, false, $6, $7, now())
             ON CONFLICT (tenant_id, operation_key) DO NOTHING",
        )
        .bind(Uuid::now_v7())
        .bind(report.tenant_id)
        .bind(report.operation_id)
        .bind(&report.target_parser.name)
        .bind(&report.target_parser.version)
        .bind(match report.status {
            ParserMigrationStatus::Partial => "partial",
            ParserMigrationStatus::Completed => "completed",
            ParserMigrationStatus::Planned => "planned",
        })
        .bind(serde_json::to_value(&report)?)
        .execute(self.reparse.pool())
        .await?;
        self.prior_report(report.tenant_id, report.operation_id)
            .await?
            .ok_or_else(|| sqlx::Error::RowNotFound.into())
    }

    async fn prior_report(
        &self,
        tenant_id: Uuid,
        operation_id: Uuid,
    ) -> Result<Option<ParserMigrationReport>, ParserMigrationError> {
        let value: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT report FROM chatgpt_archive.parser_migration_reports
             WHERE tenant_id = $1 AND operation_key = $2",
        )
        .bind(tenant_id)
        .bind(operation_id)
        .fetch_optional(self.reparse.pool())
        .await?;
        value
            .map(serde_json::from_value)
            .transpose()
            .map_err(Into::into)
    }
}
