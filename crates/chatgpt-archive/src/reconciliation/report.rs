//! Non-sensitive completeness report types.

use super::{CoverageGap, ReconciliationWarning};

/// Conservative classification of supplied archive evidence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Completeness {
    /// No positive coverage conclusion can be drawn.
    #[default]
    Unknown,
    /// Conversations are structurally incomplete or required categories lack evidence.
    StructurallyPartial,
}

/// Revision and observation statistics for one report scope.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RevisionStatistics {
    /// Revisions created while processing this archive or sequence.
    pub new_revisions: usize,
    /// Present observations which reused an existing revision.
    pub reused_revisions: usize,
    /// Present conversation observations.
    pub present_conversation_observations: usize,
    /// Missing conversation observations.
    pub missing_conversation_observations: usize,
}

/// Completeness report for one archive snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchiveCompletenessReport {
    /// Stable archive identity.
    pub archive_id: String,
    /// Detected parser schema identity.
    pub schema_id: String,
    /// Stable parser name and version formatted without source content.
    pub parser_id: String,
    /// Conversations found in this archive.
    pub conversations_discovered: usize,
    /// Messages found in this archive.
    pub messages_discovered: usize,
    /// Projects found in this archive.
    pub projects_discovered: usize,
    /// Instructions and system prompts found in this archive.
    pub instructions_discovered: usize,
    /// Canvas-like documents found in this archive.
    pub canvas_documents_discovered: usize,
    /// File or generated-asset references found in this archive.
    pub asset_references_discovered: usize,
    /// Assets with a usable verified `BlobRef`.
    pub verified_assets: usize,
    /// Assets referenced without archive bytes.
    pub missing_assets: usize,
    /// Assets retained but unavailable due to anomaly or quarantine.
    pub quarantined_assets: usize,
    /// Messages retained as graph orphans.
    pub orphan_messages: usize,
    /// Parser warnings observed while producing this archive's normalized evidence.
    pub parse_warning_count: usize,
    /// Structured non-sensitive warnings.
    pub warnings: Vec<ReconciliationWarning>,
    /// Explicit parser coverage gaps.
    pub gaps: Vec<CoverageGap>,
    /// Revision and observation counts for this archive.
    pub revision_statistics: RevisionStatistics,
    /// Conservative evidence classification.
    pub completeness: Completeness,
}

/// Aggregate evidence report across a reconciliation sequence.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CumulativeCompletenessReport {
    /// Distinct provider conversation identities.
    pub unique_conversations: usize,
    /// Distinct provider message identities.
    pub unique_messages: usize,
    /// Distinct provider project identities.
    pub unique_projects: usize,
    /// Distinct provider Canvas document identities.
    pub unique_canvas_documents: usize,
    /// Distinct provider asset identities.
    pub unique_assets: usize,
    /// Total unique conversation revisions.
    pub conversation_revisions: usize,
    /// Total unique message revisions.
    pub message_revisions: usize,
    /// Total unique project revisions.
    pub project_revisions: usize,
    /// Total unique Canvas document revisions.
    pub canvas_document_revisions: usize,
    /// Total unique asset revisions.
    pub asset_revisions: usize,
    /// Warnings from every archive in deterministic order.
    pub warnings: Vec<ReconciliationWarning>,
    /// Sum of parser warnings from every archive.
    pub parse_warning_count: usize,
    /// Coverage gaps in deterministic order.
    pub gaps: Vec<CoverageGap>,
    /// Aggregate revision and observation counts.
    pub revision_statistics: RevisionStatistics,
    /// Conservative evidence classification.
    pub completeness: Completeness,
}
