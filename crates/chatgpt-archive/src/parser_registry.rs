//! Conservative versioned parser selection.

use std::collections::BTreeSet;

use crate::ArchiveInventory;
use crate::receipt::AcquisitionMode;

/// Stable parser identity and release version.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ParserId {
    /// Stable name.
    pub name: String,
    /// Parser release.
    pub version: String,
}
/// A parser's structural and acquisition declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParserRegistration {
    /// Identity.
    pub id: ParserId,
    /// Accepted modes.
    pub modes: Vec<AcquisitionMode>,
    /// Required inventory signals.
    pub required_signals: BTreeSet<String>,
}
/// Registration refusal.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// Existing identity.
    #[error("parser identity already registered")]
    DuplicateIdentity,
}
/// Deterministic selection result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParserSelection {
    /// One match.
    Selected(ParserId),
    /// No matches.
    Unsupported,
    /// Multiple matches.
    Ambiguous(Vec<ParserId>),
}
/// Write-once parser declarations.
#[derive(Debug, Default)]
pub struct ParserRegistry {
    registrations: Vec<ParserRegistration>,
}
impl ParserRegistry {
    /// Registers a unique parser declaration.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::DuplicateIdentity`] when identity and version already exist.
    pub fn register(&mut self, registration: ParserRegistration) -> Result<(), RegistryError> {
        if self
            .registrations
            .iter()
            .any(|item| item.id == registration.id)
        {
            return Err(RegistryError::DuplicateIdentity);
        }
        self.registrations.push(registration);
        Ok(())
    }
    /// Selects exactly one declaration compatible with the inspected archive.
    #[must_use]
    pub fn select(&self, inventory: &ArchiveInventory, mode: AcquisitionMode) -> ParserSelection {
        let matches: Vec<_> = self
            .registrations
            .iter()
            .filter(|item| {
                item.modes.contains(&mode) && item.required_signals.is_subset(&inventory.signals)
            })
            .map(|item| item.id.clone())
            .collect();
        match matches.len() {
            0 => ParserSelection::Unsupported,
            1 => matches
                .first()
                .cloned()
                .map_or(ParserSelection::Unsupported, ParserSelection::Selected),
            _ => ParserSelection::Ambiguous(matches),
        }
    }
}
