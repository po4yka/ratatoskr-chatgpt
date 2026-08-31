//! The persistence seam of archive receipt.
//!
//! The receiver talks to storage only through [`ReceiptRepository`]; the
//! `PostgreSQL` implementation arrives with its own contract tests, and the
//! shared fake lives behind the `test-support` feature. Stage changes cross
//! this seam as guarded compare-and-set updates so a replayed command can
//! never regress a run.

/// The future shape the seam's async surface yields without an async-trait
/// crate.
pub type RepoFuture<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;

use uuid::Uuid;

use super::AcquisitionMode;
use super::state::ImportState;

/// A Platform operation awaiting exactly one terminal receipt report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformOperation {
    /// Platform identity supplied through the private receipt boundary.
    pub operation_id: Uuid,
}

/// Identities atomically created when raw archive evidence becomes durable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishedExport {
    /// The local raw-evidence row.
    pub export_id: Uuid,
    /// The separate Ratatoskr archive-import identity.
    pub ai_archive_id: Uuid,
}

/// All values that become durable with one raw export and its optional
/// Platform operation report.
#[derive(Debug)]
pub struct PublishRequest {
    /// The already-hashed receipt run.
    pub run_id: Uuid,
    /// Account that owns the raw evidence.
    pub account_external_ref: String,
    /// Acquisition authority of this receipt.
    pub mode: AcquisitionMode,
    /// Content-addressed blob reference for the immutable bytes.
    pub blob_ref_json: serde_json::Value,
    /// Verified lowercase SHA-256 hex.
    pub sha256_hex: String,
    /// Verified streamed byte count.
    pub byte_length: u64,
    /// The Platform operation to report, if this was a Platform receipt.
    pub platform_operation: Option<PlatformOperation>,
}

/// Why a persistence operation failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RepositoryError {
    /// The underlying store refused or could not complete the operation.
    #[error("the receipt repository failed")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// A concurrent writer changed the run between the read and the guarded
    /// update; the caller must reload before deciding anything.
    #[error("the import run changed state concurrently")]
    Conflict,
    /// The digest already exists for the tenant; the export row predates
    /// this attempt.
    #[error("an export with this digest already exists for this tenant")]
    DuplicateExisting {
        /// The identifier of the pre-existing export.
        existing_export_id: Uuid,
    },
}

impl RepositoryError {
    /// Wraps any backend failure into the erased arm.
    pub fn backend(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Backend(Box::new(source))
    }
}

/// One durable import run as the seam records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRecord {
    /// The run identity, minted by the receiver at receipt start.
    pub id: Uuid,
    /// The owning tenant's account external reference.
    pub account_external_ref: String,
    /// The declared acquisition mode spelling.
    pub acquisition_mode: String,
    /// The declared media type of the upload.
    pub media_type: String,
    /// The machine stage the run currently sits at.
    pub state: ImportState,
    /// The digest captured when the run reached `hashed`.
    pub sha256_hex: Option<String>,
    /// The byte length captured when the run reached `hashed`.
    pub byte_length: Option<i64>,
    /// The export the run produced once it reached `stored`.
    pub export_id: Option<Uuid>,
}

/// Durable receipt records: runs, exports, and their tenant scope.
pub trait ReceiptRepository: core::fmt::Debug + Send + Sync {
    /// Creates a run in state `received` for the tenant, returning its
    /// freshly minted identity.
    fn create_run(
        &self,
        account_external_ref: &str,
        mode: &AcquisitionMode,
        media_type: &str,
    ) -> RepoFuture<Result<Uuid, RepositoryError>>;

    /// Records the digest evidence and moves the run from `received` to
    /// `hashed` inside one guarded update. Fails with
    /// [`RepositoryError::Conflict`] unless the run sits at `received`.
    fn record_hash(
        &self,
        run_id: Uuid,
        sha256_hex: String,
        byte_length: u64,
    ) -> RepoFuture<Result<(), RepositoryError>>;

    /// Applies one guarded transition between arbitrary stages. Fails with
    /// [`RepositoryError::Conflict`] unless the run still sits at `expected`.
    fn mark_run(
        &self,
        run_id: Uuid,
        expected: &ImportState,
        target: ImportState,
    ) -> RepoFuture<Result<(), RepositoryError>>;

    /// Loads a run by identity, if it exists.
    fn load_run(&self, run_id: Uuid) -> RepoFuture<Result<Option<RunRecord>, RepositoryError>>;

    /// Lists the runs a previous process left non-terminal at stages this
    /// binary can resume (`received`, `hashed`), oldest first.
    fn list_resumable(&self) -> RepoFuture<Result<Vec<RunRecord>, RepositoryError>>;

    /// Finds the tenant's export holding exactly this digest.
    fn find_export_by_digest(
        &self,
        account_external_ref: &str,
        sha256_hex: &str,
    ) -> RepoFuture<Result<Option<Uuid>, RepositoryError>>;

    /// Binds a Platform operation to the import that already owns a duplicate
    /// export. The operation identity is idempotent and never creates raw
    /// evidence or a second import.
    fn bind_platform_operation(
        &self,
        existing_export_id: Uuid,
        operation: PlatformOperation,
    ) -> RepoFuture<Result<(), RepositoryError>>;

    /// Records the export and moves its run to `stored` inside one
    /// transaction, returning the new export identity. Losing the race
    /// against an equal tenant digest surfaces as
    /// [`RepositoryError::DuplicateExisting`].
    fn publish_export(
        &self,
        request: PublishRequest,
    ) -> RepoFuture<Result<PublishedExport, RepositoryError>>;
}
