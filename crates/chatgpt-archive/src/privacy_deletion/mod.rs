//! Tenant-authorized, inventory-first privacy deletion.

mod execution;
mod model;
pub(crate) mod service;

pub use model::{
    DeletionAction, DeletionInventoryItem, DeletionPlan, DeletionReport, DeletionStatus,
    PrivacyDeletionScope,
};
pub use service::{FinalizationFault, PrivacyDeletionError, PrivacyDeletionService};
