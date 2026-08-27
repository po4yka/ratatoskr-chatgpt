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
    /// Total unique conversation revisions.
    pub conversation_revisions: usize,
    /// Total unique message revisions.
    pub message_revisions: usize,
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
