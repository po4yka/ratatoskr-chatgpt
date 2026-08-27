//! Conservative reconciliation of parsed archive snapshots.

mod core;
pub(crate) mod digest;
mod model;
mod report;

pub use model::{
    ArchiveSnapshot, AssetHistory, CanvasDocumentHistory, ConversationHistory, CoverageGap,
    InstructionHistory, MessageHistory, Observation, ObservationState, ProjectHistory,
    ReconciliationResult, ReconciliationWarning, Revision, WarningCode,
};
pub use report::{
    ArchiveCompletenessReport, Completeness, CumulativeCompletenessReport, RevisionStatistics,
};

/// Reconciles ordered archive snapshots into append-only evidence.
#[derive(Debug, Default)]
pub struct ArchiveReconciler;

impl ArchiveReconciler {
    /// Reconciles present evidence into deterministic revision chains.
    #[must_use]
    pub fn reconcile(&self, snapshots: &[ArchiveSnapshot]) -> ReconciliationResult {
        core::reconcile(snapshots)
    }
}
