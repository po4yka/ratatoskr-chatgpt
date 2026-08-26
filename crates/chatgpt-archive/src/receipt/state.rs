//! The durable import state machine.
//!
//! States and transitions are declared here once; every writer — the
//! receiver, resume, later parser stages — validates through [`advance`]
//! before touching storage. Persistence applies changes as guarded
//! compare-and-set updates, so a replayed command carrying a stale source
//! stage can never regress a run.

/// One durable stage of an import run.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ImportState {
    /// Bytes are being received into staging.
    Received,
    /// The full stream arrived and its digest is recorded.
    Hashed,
    /// Bytes are durably published in blob storage and linked to an export.
    Stored,
    /// The archive passed safety inspection (later plan item).
    Inspected,
    /// Records were parsed into staging (later plan item).
    Parsed,
    /// Graphs, revisions, and assets reconciled (later plan item).
    Reconciled,
    /// Terminal: the import finished whole.
    Completed,
    /// Terminal: the import finished with recorded warnings.
    Partial,
    /// Terminal: the import failed; raw evidence survives.
    Failed,
    /// Terminal: the content duplicates an already-stored export.
    Duplicate,
    /// Terminal: reserved for safety-policy exclusion by later items.
    Quarantined,
}

/// Why a stage change was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransitionError {
    /// The run already sits at a terminal state.
    AlreadyTerminal {
        /// The terminal state that refuses all further transitions.
        current: ImportState,
    },
    /// The jump is not one the machine declares legal.
    IllegalJump {
        /// The stage the run sits at.
        current: ImportState,
        /// The stage that was requested.
        target: ImportState,
    },
}

impl ImportState {
    /// True when this state accepts no further transition.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Partial | Self::Failed | Self::Duplicate | Self::Quarantined
        )
    }

    /// The exact database spelling of this state.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Received => "received",
            Self::Hashed => "hashed",
            Self::Stored => "stored",
            Self::Inspected => "inspected",
            Self::Parsed => "parsed",
            Self::Reconciled => "reconciled",
            Self::Completed => "completed",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Duplicate => "duplicate",
            Self::Quarantined => "quarantined",
        }
    }

    /// Parses the database spelling back into a state.
    #[must_use]
    pub fn parse(spelling: &str) -> Option<Self> {
        let state = match spelling {
            "received" => Self::Received,
            "hashed" => Self::Hashed,
            "stored" => Self::Stored,
            "inspected" => Self::Inspected,
            "parsed" => Self::Parsed,
            "reconciled" => Self::Reconciled,
            "completed" => Self::Completed,
            "partial" => Self::Partial,
            "failed" => Self::Failed,
            "duplicate" => Self::Duplicate,
            "quarantined" => Self::Quarantined,
            _ => return None,
        };
        Some(state)
    }

    /// Moves this run to `target` when the machine allows it, returning the
    /// reached state; otherwise a typed refusal naming the reason.
    ///
    /// Legal moves: one step of `received -> hashed -> stored -> inspected ->
    /// parsed -> reconciled`, then either terminal success class;
    /// `hashed -> duplicate` when the digest check fires; `failed` from any
    /// non-terminal stage. Terminal stages accept nothing.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] when the current stage is terminal or the
    /// jump is not one the machine declares legal.
    pub fn advance(self, target: ImportState) -> Result<ImportState, TransitionError> {
        if self.is_terminal() {
            return Err(TransitionError::AlreadyTerminal { current: self });
        }
        let legal = matches!(
            (&self, &target),
            (Self::Received, Self::Hashed)
                | (Self::Hashed, Self::Stored | Self::Duplicate)
                | (Self::Stored, Self::Inspected)
                | (Self::Inspected, Self::Parsed)
                | (Self::Parsed, Self::Reconciled)
                | (Self::Reconciled, Self::Completed | Self::Partial)
                | (_, Self::Failed)
        );
        if legal {
            Ok(target)
        } else {
            Err(TransitionError::IllegalJump {
                current: self,
                target,
            })
        }
    }
}
