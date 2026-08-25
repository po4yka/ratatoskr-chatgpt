#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Domain library for the Ratatoskr `ChatGPT` bounded context.
//!
//! The foundation owns process configuration, telemetry bootstrap, the error
//! taxonomy and its single HTTP rendering site, the operator admin plane,
//! content-addressed blob storage under the fleet `BlobRef` contract, and
//! application of the first-version `chatgpt_archive` schema. Archive
//! receipt, safe extraction, parsers, graph reconciliation, and portable
//! exports arrive with later implementation plan items.

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

pub mod persistence;

pub use persistence::{Database, PersistenceError};
