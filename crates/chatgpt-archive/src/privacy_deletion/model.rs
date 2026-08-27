use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One tenant-owned privacy deletion scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PrivacyDeletionScope {
    /// One immutable archive and evidence supported only by it.
    Archive {
        /// Stable AI-archive identity from the fleet contract.
        ai_archive_id: Uuid,
    },
    /// One conversation and every raw archive containing it.
    Conversation {
        /// Archive-owned normalized conversation identity.
        conversation_id: Uuid,
    },
    /// Every archive-owned record for the authenticated tenant.
    Tenant,
}

/// Planned treatment of one opaque inventory identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeletionAction {
    /// Erase an exclusive archive-owned blob.
    Erase,
    /// Remove a database record during atomic finalization.
    Remove,
    /// Keep bytes still referenced by retained evidence.
    RetainShared,
    /// Keep normalized state proven by a retained raw archive.
    RetainEvidenced,
    /// Emit an authoritative downstream deletion tombstone.
    EmitTombstone,
}

/// One content-free item in a deterministic deletion inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletionInventoryItem {
    /// Stable category name without content or provider labels.
    pub category: String,
    /// Opaque internal identity.
    pub opaque_id: String,
    /// Planned treatment.
    pub action: DeletionAction,
}

/// Persisted preflight result for a single request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletionPlan {
    /// Stable replay identity.
    pub request_id: Uuid,
    /// Authenticated internal tenant identity.
    pub tenant_id: Uuid,
    /// Authorized deletion scope.
    pub scope: PrivacyDeletionScope,
    /// Items sorted by category, opaque identity, and action.
    pub items: Vec<DeletionInventoryItem>,
    /// Per-category totals derived only from `items`.
    pub totals: BTreeMap<String, u64>,
}

impl DeletionPlan {
    pub(super) fn new(
        request_id: Uuid,
        tenant_id: Uuid,
        scope: PrivacyDeletionScope,
        mut items: Vec<DeletionInventoryItem>,
    ) -> Self {
        items.sort_by(|left, right| {
            (&left.category, &left.opaque_id, left.action).cmp(&(
                &right.category,
                &right.opaque_id,
                right.action,
            ))
        });
        let mut totals = BTreeMap::new();
        for item in &items {
            *totals.entry(item.category.clone()).or_insert(0) += 1;
        }
        Self {
            request_id,
            tenant_id,
            scope,
            items,
            totals,
        }
    }
}

impl DeletionAction {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Erase => "erase",
            Self::Remove => "remove",
            Self::RetainShared => "retain_shared",
            Self::RetainEvidenced => "retain_evidenced",
            Self::EmitTombstone => "emit_tombstone",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "erase" => Some(Self::Erase),
            "remove" => Some(Self::Remove),
            "retain_shared" => Some(Self::RetainShared),
            "retain_evidenced" => Some(Self::RetainEvidenced),
            "emit_tombstone" => Some(Self::EmitTombstone),
            _ => None,
        }
    }
}

/// Terminal privacy deletion outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeletionStatus {
    /// Every exclusive byte and selected database record was removed.
    Completed,
}

/// Stable, content-free result returned on completion and replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletionReport {
    /// Stable request identity.
    pub request_id: Uuid,
    /// Terminal outcome.
    pub status: DeletionStatus,
    /// Counts copied from the executed inventory.
    pub totals: BTreeMap<String, u64>,
    /// Content-free evidence blob retained for audit.
    pub evidence_ref: ratatoskr_identifiers::BlobRef,
    /// Stable database completion instant in RFC 3339 form.
    pub completed_at: String,
}
