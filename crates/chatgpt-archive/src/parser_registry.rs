//! Conservative versioned parser selection.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::sync::Arc;

use bytes::Bytes;

use crate::receipt::AcquisitionMode;
use crate::{ArchiveInventory, ParsedConversations};

/// Stable parser identity and release version.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
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

/// Verified extracted evidence handed to a compiled parser implementation.
#[derive(Debug, Clone, Copy)]
pub struct ParserExecutionInput<'a> {
    /// Hostile-input-safe archive inventory.
    pub inventory: &'a ArchiveInventory,
    /// Bounded, re-inspected entry evidence held only for this execution.
    pub artifacts: &'a [ParserArtifactEvidence],
}

/// One bounded archive entry made inert and available to a compiled parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParserArtifactEvidence {
    /// Normalized archive-relative path.
    pub path: String,
    /// Exact entry bytes read under current hostile-input limits.
    pub bytes: Bytes,
    /// Whether policy prevents this entry from becoming trusted input.
    pub quarantined: bool,
}

/// A compiled parser failure that does not expose archive content.
#[derive(Debug, thiserror::Error)]
pub enum ParserExecutionError {
    /// The parser rejected or could not normalize the supplied evidence.
    #[error("compiled archive parser failed")]
    Failed,
}

/// Executable boundary paired with one parser declaration.
pub trait ParserExecutor: core::fmt::Debug + Send + Sync {
    /// Parses verified evidence into the existing normalized parser result.
    ///
    /// # Errors
    ///
    /// Returns [`ParserExecutionError`] without embedding source content.
    fn execute(
        &self,
        input: ParserExecutionInput<'_>,
    ) -> Result<ParsedConversations, ParserExecutionError>;
}

/// One exact compatible compiled parser resolved for operator execution.
#[derive(Debug, Clone)]
pub struct CompiledParser {
    /// Stable declared identity.
    pub id: ParserId,
    executor: Arc<dyn ParserExecutor>,
}

impl CompiledParser {
    /// Executes this exact parser over verified evidence.
    ///
    /// # Errors
    ///
    /// Returns the parser's content-free failure.
    pub fn execute(
        &self,
        input: ParserExecutionInput<'_>,
    ) -> Result<ParsedConversations, ParserExecutionError> {
        self.executor.execute(input)
    }
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
    registrations: Vec<RegisteredParser>,
}

#[derive(Debug)]
struct RegisteredParser {
    declaration: ParserRegistration,
    executor: Option<Arc<dyn ParserExecutor>>,
}
impl ParserRegistry {
    /// Builds the compiled parser set used by the service runtime.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError`] if the built-in declarations overlap.
    pub fn runtime() -> Result<Self, RegistryError> {
        let mut registry = Self::default();
        registry.register_compiled(
            crate::SyntheticConversationsParser::registration(),
            Arc::new(crate::SyntheticConversationsParser),
        )?;
        Ok(registry)
    }

    /// Registers a unique parser declaration.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::DuplicateIdentity`] when identity and version already exist.
    pub fn register(&mut self, registration: ParserRegistration) -> Result<(), RegistryError> {
        if self
            .registrations
            .iter()
            .any(|item| item.declaration.id == registration.id)
        {
            return Err(RegistryError::DuplicateIdentity);
        }
        self.registrations.push(RegisteredParser {
            declaration: registration,
            executor: None,
        });
        Ok(())
    }

    /// Registers a unique declaration together with compiled behavior.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::DuplicateIdentity`] for an existing identity.
    pub fn register_compiled(
        &mut self,
        registration: ParserRegistration,
        executor: Arc<dyn ParserExecutor>,
    ) -> Result<(), RegistryError> {
        if self
            .registrations
            .iter()
            .any(|item| item.declaration.id == registration.id)
        {
            return Err(RegistryError::DuplicateIdentity);
        }
        self.registrations.push(RegisteredParser {
            declaration: registration,
            executor: Some(executor),
        });
        Ok(())
    }

    /// Lists every compatible identity in deterministic declared-version order.
    #[must_use]
    pub fn compatible_versions(
        &self,
        inventory: &ArchiveInventory,
        mode: AcquisitionMode,
    ) -> Vec<ParserId> {
        let mut compatible: Vec<_> = self
            .registrations
            .iter()
            .filter(|item| compatible(&item.declaration, inventory, mode))
            .map(|item| item.declaration.id.clone())
            .collect();
        compatible.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| compare_versions(&left.version, &right.version))
        });
        compatible
    }

    /// Resolves one exact compatible compiled identity without auto-selection.
    #[must_use]
    pub fn find_exact(
        &self,
        id: &ParserId,
        inventory: &ArchiveInventory,
        mode: AcquisitionMode,
    ) -> Option<CompiledParser> {
        self.registrations.iter().find_map(|item| {
            (item.declaration.id == *id && compatible(&item.declaration, inventory, mode))
                .then(|| {
                    item.executor.as_ref().map(|executor| CompiledParser {
                        id: item.declaration.id.clone(),
                        executor: Arc::clone(executor),
                    })
                })
                .flatten()
        })
    }
    /// Selects exactly one declaration compatible with the inspected archive.
    #[must_use]
    pub fn select(&self, inventory: &ArchiveInventory, mode: AcquisitionMode) -> ParserSelection {
        let mut matches: Vec<_> = self
            .registrations
            .iter()
            .filter(|item| compatible(&item.declaration, inventory, mode))
            .map(|item| item.declaration.id.clone())
            .collect();
        matches.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| compare_versions(&left.version, &right.version))
        });
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

fn compatible(
    registration: &ParserRegistration,
    inventory: &ArchiveInventory,
    mode: AcquisitionMode,
) -> bool {
    registration.modes.contains(&mode)
        && registration.required_signals.is_subset(&inventory.signals)
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum VersionPart<'a> {
    Number(u64),
    Text(&'a str),
}

fn compare_versions(left: &str, right: &str) -> Ordering {
    version_parts(left).cmp(&version_parts(right))
}

fn version_parts(version: &str) -> Vec<VersionPart<'_>> {
    version
        .split(['.', '-', '_'])
        .map(|part| {
            part.parse::<u64>()
                .map_or(VersionPart::Text(part), VersionPart::Number)
        })
        .collect()
}
