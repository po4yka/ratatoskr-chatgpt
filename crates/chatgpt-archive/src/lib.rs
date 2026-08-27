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
//! version from structural evidence. A narrow synthetic conversations parser
//! produces deterministic, loss-aware records without claiming real-export
//! support; graph reconciliation and portable exports arrive with later plan
//! items.

pub mod config;

pub use config::{AdminConfig, Config, ConfigError, Limits, StorageConfig, TelemetryConfig};

pub mod telemetry;

pub use telemetry::{TelemetryError, TelemetryGuard, init_telemetry};

pub mod error;
pub mod fault;

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

pub mod parser_registry;
pub mod receipt;
pub mod synthetic_parser;
pub use parser_registry::{
    ParserId, ParserRegistration, ParserRegistry, ParserSelection, RegistryError,
};
pub use synthetic_parser::{
    ContentPartKind, MessageRole, ParsedContentPart, ParsedConversation, ParsedConversations,
    ParsedMessage, RawRecord, SYNTHETIC_PARSER_NAME, SYNTHETIC_PARSER_VERSION, SYNTHETIC_SCHEMA_ID,
    SyntheticConversationsParser, SyntheticParserError,
};

#[cfg(feature = "test-support")]
pub mod test_support;

pub use receipt::{
    AcquisitionMode, ArchiveReceiver, ReceiptError, ReceiptOutcome, ReceiptRepository,
    RepositoryError, RunRecord,
};
