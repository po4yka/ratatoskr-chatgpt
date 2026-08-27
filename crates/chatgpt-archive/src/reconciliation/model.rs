//! Public evidence types for archive reconciliation.

use super::{ArchiveCompletenessReport, CumulativeCompletenessReport};

/// One archive snapshot supplied to reconciliation in chronological order.
#[derive(Debug, Clone, PartialEq)]
pub struct ArchiveSnapshot {
    /// Stable, non-sensitive archive identity supplied by the caller.
    pub archive_id: String,
    /// Strictly increasing position in the supplied archive sequence.
    pub sequence: u64,
    /// Parsed normalized evidence for this archive.
    pub parsed: crate::ParsedConversations,
}

/// Full append-only reconciliation result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconciliationResult {
    /// Histories ordered by conversation external ID.
    pub conversations: Vec<ConversationHistory>,
    /// One report per supplied archive in sequence order.
    pub archive_reports: Vec<ArchiveCompletenessReport>,
    /// Aggregate report across every supplied archive.
    pub cumulative_report: CumulativeCompletenessReport,
}

/// Append-only evidence for one provider conversation identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationHistory {
    /// Stable provider conversation identity.
    pub external_id: String,
    /// Distinct normalized conversation revisions in first-seen order.
    pub revisions: Vec<Revision>,
    /// Archive observations in sequence order.
    pub observations: Vec<Observation>,
    /// Message histories ordered by provider message identity.
    pub messages: Vec<MessageHistory>,
}

/// Append-only evidence for one provider message identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageHistory {
    /// Stable provider message identity.
    pub external_id: String,
    /// Distinct normalized message revisions in first-seen order.
    pub revisions: Vec<Revision>,
    /// Archive observations in sequence order.
    pub observations: Vec<Observation>,
}

/// One normalized record revision addressed by its content digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    /// Lowercase SHA-256 digest of canonical normalized evidence.
    pub digest: String,
}

/// One archive's observation of an identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// Archive which supplied or omitted this identity.
    pub archive_id: String,
    /// Observed evidence state.
    pub state: ObservationState,
    /// Referenced revision digest for a present observation.
    pub revision_digest: Option<String>,
    /// Whether graph validation retained this message as an orphan.
    pub orphaned: bool,
}

/// Non-destructive archive observation states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationState {
    /// The archive contains the identity and references a revision.
    Present,
    /// A previously observed conversation was omitted from this archive.
    MissingFromLatestSnapshot,
}

/// Non-sensitive graph warning associated with an archive report.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReconciliationWarning {
    /// Stable warning classification without source content.
    pub code: WarningCode,
    /// Stable conversation identity associated with the warning.
    pub conversation_external_id: String,
    /// Stable message identity associated with the warning when applicable.
    pub message_external_id: Option<String>,
}

/// Classified graph warning that does not disclose provider content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WarningCode {
    /// A message parent was not present in its conversation.
    MissingParent,
    /// A message named itself as parent.
    SelfParent,
    /// A parent ID was known only from another conversation.
    CrossConversationParent,
}

/// Explicit coverage limitation for a supported parser path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CoverageGap {
    /// The parser supplied no project or membership relationship evidence.
    ProjectRelationshipsUnobserved,
    /// The parser supplied no archived asset-byte evidence.
    AssetsUnobserved,
}
