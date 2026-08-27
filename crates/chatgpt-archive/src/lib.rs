#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Domain library for the Ratatoskr `ChatGPT` bounded context.
//!
//! The foundation owns process configuration, telemetry bootstrap, the error
//! taxonomy and its single HTTP rendering site, the operator admin plane,
//! content-addressed blob storage under the fleet `BlobRef` contract, and
//! application of the first-version `chatgpt_archive` schema. Archive
//! receipt (plan item 2) adds the authenticated, tenant-scoped `POST
//! /exports` surface: streaming SHA-256 into isolated staging, size caps,
//! immutable raw evidence through the fleet [`BlobStore`], duplicate outcomes
//! by per-tenant digest, and a durable resumable import state machine. Safe
//! archive inspection and bounded extraction now preserve every accepted entry
//! through immutable `BlobRefs`, while parser registration selects a supported
//! version from structural evidence. A narrow synthetic archive parser produces
//! deterministic, loss-aware conversation, project, Canvas, and asset evidence
//! without claiming real-export support. Asset bytes remain usable only after
//! `BlobRef` verification and exact provider-declaration checks; anomalies remain
//! quarantined without rendering or preview. The public reconciliation boundary
//! then builds append-only revisions, non-destructive missing observations,
//! graph warnings, and conservative in-memory completeness reports. It is not
//! yet wired into receipt persistence or outbound archive events.

pub mod config;

pub use config::{AdminConfig, Config, ConfigError, Limits, StorageConfig, TelemetryConfig};

pub mod telemetry;

pub use telemetry::{TelemetryError, TelemetryGuard, init_telemetry};

pub mod error;
pub mod fault;
pub mod fixture_admission;

pub use error::{ArchiveError, FailureKind, PublicFault, Subsystem};
pub use fault::render_error_envelope;

pub mod admin;

pub mod blob_store;

pub use blob_store::{BlobStore, BlobStoreError};

pub mod archive;

pub use archive::{
    ArchiveEntry, ArchiveExtractor, ArchiveInspector, ArchiveIntakeError, ArchiveInventory,
    ArchiveLimits, ArtifactProvenance, EntryKind, ExtractedArtifact,
};

pub mod persistence;

pub use persistence::{Database, PersistenceError};

pub mod outbox;
pub mod parser_migration;
pub mod parser_registry;
pub mod portable_export;
pub mod privacy_deletion;
pub mod receipt;
pub mod reconciliation;
pub mod reparse;
pub mod synthetic_parser;
pub use parser_registry::{
    CompiledParser, ParserArtifactEvidence, ParserExecutionError, ParserExecutionInput,
    ParserExecutor, ParserId, ParserRegistration, ParserRegistry, ParserSelection, RegistryError,
};
pub use synthetic_parser::{
    AssetAnomaly, AssetAvailability, AssetKind, ContentPartKind, InstructionKind, MessageRole,
    ParsedAsset, ParsedCanvasDocument, ParsedContentPart, ParsedConversation, ParsedConversations,
    ParsedInstruction, ParsedMessage, ParsedProject, RawRecord, SYNTHETIC_PARSER_NAME,
    SYNTHETIC_PARSER_VERSION, SYNTHETIC_SCHEMA_ID, SyntheticArchiveInput,
    SyntheticConversationsParser, SyntheticParserError,
};

#[cfg(feature = "test-support")]
pub mod test_support;

pub use outbox::{NormalizedArchiveEvent, OutboxError};
pub use receipt::{
    AcquisitionMode, ArchiveReceiver, ReceiptError, ReceiptOutcome, ReceiptRepository,
    RepositoryError, RunRecord,
};
pub use reconciliation::{
    ArchiveCompletenessReport, ArchiveReconciler, ArchiveSnapshot, AssetHistory,
    CanvasDocumentHistory, Completeness, ConversationHistory, CoverageGap,
    CumulativeCompletenessReport, InstructionHistory, MessageHistory, Observation,
    ObservationState, ProjectHistory, ReconciliationResult, ReconciliationWarning, Revision,
    RevisionStatistics, WarningCode,
};
