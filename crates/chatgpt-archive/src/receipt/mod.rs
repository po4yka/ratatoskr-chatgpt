//! Archive receipt: authenticated, tenant-scoped intake of export archives.
//!
//! The module tree owns the receipt surface (plan item 2): the durable
//! import state machine, tenant authentication, the streaming receiver, and
//! its persistence seam. The HTTP surface lives beside them.

pub mod auth;
pub mod http;
pub mod outbox;
pub mod pg;
pub(crate) mod report;
pub mod repository;
pub mod state;

use crate::receipt::state::ImportState;

pub use outbox::OperationReportOutbox;
pub use repository::{
    PlatformOperation, PublishRequest, PublishedExport, ReceiptRepository, RepositoryError,
    RunRecord,
};

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use futures_core::Stream;
use uuid::Uuid;

/// How this archive arrived. Each mode has different authority and
/// completeness; they are never merged into an unlabeled import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AcquisitionMode {
    /// An official personal-account export.
    ConsumerExport,
    /// An official education-workspace export.
    EduExport,
    /// A compliance log feed.
    ComplianceLog,
    /// A manually captured conversation; partial by definition.
    ManualCapture,
    /// Data brought in from a previous system.
    LegacyImport,
}

impl AcquisitionMode {
    /// The exact database spelling of this mode.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ConsumerExport => "consumer_export",
            Self::EduExport => "edu_export",
            Self::ComplianceLog => "compliance_log",
            Self::ManualCapture => "manual_capture",
            Self::LegacyImport => "legacy_import",
        }
    }

    /// Parses the database spelling back into a mode.
    #[must_use]
    pub fn parse(spelling: &str) -> Option<Self> {
        let mode = match spelling {
            "consumer_export" => Self::ConsumerExport,
            "edu_export" => Self::EduExport,
            "compliance_log" => Self::ComplianceLog,
            "manual_capture" => Self::ManualCapture,
            "legacy_import" => Self::LegacyImport,
            _ => return None,
        };
        Some(mode)
    }
}

/// What one completed receipt produced.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReceiptOutcome {
    /// New evidence: the bytes were published and recorded under this id.
    Stored {
        /// The fresh export identity.
        export_id: Uuid,
        /// Separately minted Ratatoskr archive-import identity.
        ai_archive_id: Uuid,
        /// The lowercase SHA-256 hex of the received bytes.
        sha256_hex: String,
        /// Total bytes received.
        byte_length: u64,
    },
    /// Same content: an equal digest already existed for this tenant.
    Duplicate {
        /// The identifier of the pre-existing export.
        existing_export_id: Uuid,
        /// The lowercase SHA-256 hex of the received bytes.
        sha256_hex: String,
        /// Total bytes received.
        byte_length: u64,
    },
}

/// Why an upload stream failed before completion.
#[derive(Debug)]
pub struct StreamFailure(pub Box<dyn std::error::Error + Send + Sync>);

impl core::fmt::Display for StreamFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for StreamFailure {}

/// Why a receipt did not complete.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReceiptError {
    /// The declared `Content-Length` alone exceeds the cap; nothing streamed.
    #[error("the declared archive size exceeds the configured maximum")]
    DeclaredSizeExceeded,
    /// The running byte total crossed the cap mid-stream; consumption stops.
    #[error("the archive exceeded the configured maximum while streaming")]
    StreamOvergrown,
    /// The stream itself failed before delivering the declared body.
    #[error("the upload stream failed before completion")]
    StreamFailed(#[source] StreamFailure),
    /// Staging or blob storage refused the work.
    #[error("receipt storage failed")]
    Storage(#[source] std::io::Error),
    /// Durable records could not be written or advanced.
    #[error("the receipt could not be recorded")]
    Repository(#[source] RepositoryError),
    /// The staged bytes a non-terminal run needs are gone or fail
    /// verification; resume refuses to invent evidence.
    #[error("the staging evidence for this run is lost")]
    StagingEvidenceLost,
    /// The declared media type is not `type/subtype`.
    #[error("the declared media type is invalid")]
    InvalidMediaType,
    /// The streamed bytes differ from the trusted archive digest.
    #[error("the received archive digest did not match the declared identity")]
    DigestMismatch,
}

/// Streaming archive receiver.
///
/// Chunks are hashed and staged as they arrive — memory stays bounded by one
/// chunk — then published through [`BlobStore`] only once the digest proves
/// the content is not a duplicate for this tenant.
#[derive(core::fmt::Debug)]
pub struct ArchiveReceiver {
    blob: crate::blob_store::BlobStore,
    repository: Arc<dyn ReceiptRepository>,
    staging_root: PathBuf,
    max_archive_bytes: u64,
}

/// Trusted expectations attached to one receipt after its boundary validated
/// Platform claims. Keeping them together prevents a new trusted claim from
/// becoming another positional parameter in the streaming state machine.
#[derive(Debug, Clone, Copy)]
struct ReceiptExpectations<'a> {
    expected_sha256_hex: Option<&'a str>,
    platform_operation: Option<PlatformOperation>,
}

impl<'a> ReceiptExpectations<'a> {
    const fn none() -> Self {
        Self {
            expected_sha256_hex: None,
            platform_operation: None,
        }
    }

    const fn with_digest(expected_sha256_hex: &'a str) -> Self {
        Self {
            expected_sha256_hex: Some(expected_sha256_hex),
            platform_operation: None,
        }
    }

    const fn platform(expected_sha256_hex: &'a str, operation: PlatformOperation) -> Self {
        Self {
            expected_sha256_hex: Some(expected_sha256_hex),
            platform_operation: Some(operation),
        }
    }
}

impl ArchiveReceiver {
    /// Creates a receiver rooted at an isolated staging directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the staging root cannot be created with
    /// restrictive permissions.
    pub fn new(
        blob: crate::blob_store::BlobStore,
        repository: Arc<dyn ReceiptRepository>,
        staging_root: PathBuf,
        max_archive_bytes: u64,
    ) -> Result<Self, ReceiptError> {
        std::fs::create_dir_all(&staging_root).map_err(ReceiptError::Storage)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let permissions = std::fs::Permissions::from_mode(0o700);
            std::fs::set_permissions(&staging_root, permissions).map_err(ReceiptError::Storage)?;
        }
        Ok(Self {
            blob,
            repository,
            staging_root,
            max_archive_bytes,
        })
    }

    fn staging_path(&self, run_id: Uuid) -> PathBuf {
        // The file name is derived from the minted run identity only; no
        // client-supplied string ever becomes a path component.
        self.staging_root.join(format!("{run_id}.part"))
    }

    /// Marks a run `failed` from `expected`, swallowing only the refusal of
    /// a concurrent writer who already recorded an outcome.
    async fn fail_run(&self, run_id: Uuid, expected: &ImportState) {
        let _ = self
            .repository
            .mark_run(run_id, expected, ImportState::Failed)
            .await;
    }

    /// Receives one upload to completion.
    ///
    /// # Errors
    ///
    /// Returns [`ReceiptError`] for oversized declarations, oversize or
    /// truncation discovered mid-stream, staging failures, and persistence
    /// refusals. Every durable anchor that exists when the failure happens
    /// ends in the terminal `failed` state.
    pub async fn receive<S, E>(
        &self,
        principal: &crate::receipt::auth::TenantPrincipal,
        mode: AcquisitionMode,
        media_type: &str,
        declared_length: Option<u64>,
        stream: S,
    ) -> Result<ReceiptOutcome, ReceiptError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin + Send + 'static,
        E: std::error::Error + Send + Sync + 'static,
    {
        self.receive_inner(
            principal,
            mode,
            media_type,
            declared_length,
            ReceiptExpectations::none(),
            stream,
        )
        .await
    }

    /// Receives one archive after a trusted boundary fixed its digest.
    ///
    /// # Errors
    ///
    /// Returns [`ReceiptError::DigestMismatch`] before raw publication when
    /// bytes disagree with the declared archive identity.
    pub async fn receive_with_expected_digest<S, E>(
        &self,
        principal: &crate::receipt::auth::TenantPrincipal,
        mode: AcquisitionMode,
        media_type: &str,
        declared_length: Option<u64>,
        expected_sha256_hex: &str,
        stream: S,
    ) -> Result<ReceiptOutcome, ReceiptError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin + Send + 'static,
        E: std::error::Error + Send + Sync + 'static,
    {
        self.receive_inner(
            principal,
            mode,
            media_type,
            declared_length,
            ReceiptExpectations::with_digest(expected_sha256_hex),
            stream,
        )
        .await
    }

    /// Receives Platform-forwarded bytes and durably binds non-terminal import work.
    ///
    /// # Errors
    ///
    /// Returns [`ReceiptError`] when claims, bytes, storage, or durable
    /// reporting cannot be completed safely.
    pub async fn receive_platform_archive<S, E>(
        &self,
        principal: &crate::receipt::auth::TenantPrincipal,
        media_type: &str,
        declared_length: u64,
        expected_sha256_hex: &str,
        operation: PlatformOperation,
        stream: S,
    ) -> Result<ReceiptOutcome, ReceiptError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin + Send + 'static,
        E: std::error::Error + Send + Sync + 'static,
    {
        self.receive_inner(
            principal,
            AcquisitionMode::ConsumerExport,
            media_type,
            Some(declared_length),
            ReceiptExpectations::platform(expected_sha256_hex, operation),
            stream,
        )
        .await
    }

    async fn receive_inner<S, E>(
        &self,
        principal: &crate::receipt::auth::TenantPrincipal,
        mode: AcquisitionMode,
        media_type: &str,
        declared_length: Option<u64>,
        expectations: ReceiptExpectations<'_>,
        stream: S,
    ) -> Result<ReceiptOutcome, ReceiptError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin + Send + 'static,
        E: std::error::Error + Send + Sync + 'static,
    {
        ratatoskr_identifiers::MediaType::parse(media_type)
            .map_err(|_| ReceiptError::InvalidMediaType)?;

        // The durable anchor exists before the first body byte is read.
        let run_id = self
            .repository
            .create_run(&principal.account_external_ref, &mode, media_type)
            .await
            .map_err(ReceiptError::Repository)?;

        if declared_length.is_some_and(|declared| declared > self.max_archive_bytes) {
            self.fail_run(run_id, &ImportState::Received).await;
            return Err(ReceiptError::DeclaredSizeExceeded);
        }

        let staging_path = self.staging_path(run_id);
        let staged = stream_to_staging(&staging_path, stream, self.max_archive_bytes).await;
        let (sha256_hex, byte_length) = match staged {
            Ok(evidence) => evidence,
            Err(failure) => {
                let _ = tokio::fs::remove_file(&staging_path).await;
                self.fail_run(run_id, &ImportState::Received).await;
                return Err(match failure {
                    StagingFailure::Overgrown => ReceiptError::StreamOvergrown,
                    StagingFailure::Stream(source) => ReceiptError::StreamFailed(source),
                    StagingFailure::Io(error) => ReceiptError::Storage(error),
                });
            }
        };

        // A clean close that delivered less than declared is truncation too.
        if declared_length.is_some_and(|declared| declared != byte_length) {
            let _ = tokio::fs::remove_file(&staging_path).await;
            self.fail_run(run_id, &ImportState::Received).await;
            return Err(ReceiptError::StreamFailed(StreamFailure(Box::new(
                std::io::Error::other("the body ended before its declared length"),
            ))));
        }

        if expectations
            .expected_sha256_hex
            .is_some_and(|expected| expected != sha256_hex)
        {
            let _ = tokio::fs::remove_file(&staging_path).await;
            self.fail_run(run_id, &ImportState::Received).await;
            return Err(ReceiptError::DigestMismatch);
        }

        self.repository
            .record_hash(run_id, sha256_hex.clone(), byte_length)
            .await
            .map_err(ReceiptError::Repository)?;

        let context = HashedContext {
            run_id,
            tenant: principal.account_external_ref.clone(),
            mode_spelling: mode.as_str().to_owned(),
            media_type: media_type.to_owned(),
            sha256_hex,
            byte_length,
        };
        let outcome = self
            .settle_hashed(context, expectations.platform_operation, &staging_path)
            .await;
        let _ = tokio::fs::remove_file(&staging_path).await;
        match &outcome {
            Ok(ReceiptOutcome::Stored { .. }) => {
                metrics::counter!(
                    "chatgpt_archive_receipts_total",
                    "outcome" => "stored"
                )
                .increment(1);
                metrics::counter!("chatgpt_archive_receipt_bytes").increment(byte_length);
            }
            Ok(ReceiptOutcome::Duplicate { .. }) => {
                metrics::counter!(
                    "chatgpt_archive_receipts_total",
                    "outcome" => "duplicate"
                )
                .increment(1);
            }
            Err(_) => {}
        }
        outcome
    }

    /// Walks a hashed run to its terminal class: duplicate check, publish,
    /// export recording.
    async fn settle_hashed(
        &self,
        context: HashedContext,
        platform_operation: Option<PlatformOperation>,
        staging_path: &Path,
    ) -> Result<ReceiptOutcome, ReceiptError> {
        if let Some(existing_export_id) = self
            .repository
            .find_export_by_digest(&context.tenant, &context.sha256_hex)
            .await
            .map_err(ReceiptError::Repository)?
        {
            if let Some(operation) = platform_operation {
                self.repository
                    .bind_platform_operation(existing_export_id, operation)
                    .await
                    .map_err(ReceiptError::Repository)?;
            }
            self.repository
                .mark_run(context.run_id, &ImportState::Hashed, ImportState::Duplicate)
                .await
                .map_err(ReceiptError::Repository)?;
            return Ok(ReceiptOutcome::Duplicate {
                existing_export_id,
                sha256_hex: context.sha256_hex,
                byte_length: context.byte_length,
            });
        }

        let blob_ref_json = self
            .publish_staged(&context.media_type, staging_path)
            .await?;

        match self
            .repository
            .publish_export(PublishRequest {
                run_id: context.run_id,
                account_external_ref: context.tenant.clone(),
                mode: AcquisitionMode::parse(&context.mode_spelling)
                    .unwrap_or(AcquisitionMode::ConsumerExport),
                blob_ref_json,
                sha256_hex: context.sha256_hex.clone(),
                byte_length: context.byte_length,
                platform_operation,
            })
            .await
        {
            Ok(published) => Ok(ReceiptOutcome::Stored {
                export_id: published.export_id,
                ai_archive_id: published.ai_archive_id,
                sha256_hex: context.sha256_hex,
                byte_length: context.byte_length,
            }),
            Err(RepositoryError::DuplicateExisting { existing_export_id }) => {
                if let Some(operation) = platform_operation {
                    self.repository
                        .bind_platform_operation(existing_export_id, operation)
                        .await
                        .map_err(ReceiptError::Repository)?;
                }
                self.repository
                    .mark_run(context.run_id, &ImportState::Hashed, ImportState::Duplicate)
                    .await
                    .map_err(ReceiptError::Repository)?;
                Ok(ReceiptOutcome::Duplicate {
                    existing_export_id,
                    sha256_hex: context.sha256_hex,
                    byte_length: context.byte_length,
                })
            }
            Err(other) => Err(ReceiptError::Repository(other)),
        }
    }

    /// Publishes staged bytes through the content-addressed store by
    /// streaming them from disk in bounded chunks.
    async fn publish_staged(
        &self,
        media_type: &str,
        staging_path: &Path,
    ) -> Result<serde_json::Value, ReceiptError> {
        use tokio::io::AsyncReadExt as _;

        let file = tokio::fs::File::open(staging_path)
            .await
            .map_err(ReceiptError::Storage)?;
        let chunk_size = 64 * 1024;
        let stream = futures_util::stream::unfold(
            (file, vec![0u8; chunk_size]),
            |(mut file, mut buffer)| async move {
                match file.read(&mut buffer).await {
                    Ok(0) => None,
                    Ok(read) => match buffer.get(..read) {
                        Some(bytes) => Some((
                            Ok::<Bytes, std::io::Error>(Bytes::copy_from_slice(bytes)),
                            (file, buffer),
                        )),
                        None => Some((
                            Err(std::io::Error::other(
                                "the reader reported a length outside the buffer",
                            )),
                            (file, buffer),
                        )),
                    },
                    Err(error) => Some((Err(error), (file, buffer))),
                }
            },
        );
        // The store's seam requires an `Unpin` stream; boxing the unfold
        // provides it without buffering more than one chunk.
        let stream = Box::pin(stream);
        let blob_ref = self
            .blob
            .store(media_type, stream)
            .await
            .map_err(|error| ReceiptError::Storage(std::io::Error::other(error)))?;
        serde_json::to_value(&blob_ref)
            .map_err(|error| ReceiptError::Storage(std::io::Error::other(error)))
    }

    /// Re-hashes a staging file, refusing lost or unverifiable evidence.
    ///
    /// # Errors
    ///
    /// [`ReceiptError::StagingEvidenceLost`] when the file is absent or does
    /// not hash as expected; [`ReceiptError::Storage`] on IO failure.
    async fn verify_staged(
        &self,
        run_id: Uuid,
        staging_path: &Path,
        expected_hex: &str,
        expected_length: u64,
    ) -> Result<(), ReceiptError> {
        let hashed = hash_file(staging_path).await;
        match hashed {
            Ok((actual_hex, actual_length))
                if actual_hex == expected_hex && actual_length == expected_length =>
            {
                Ok(())
            }
            Ok(_) => {
                // Evidence drifted from what was recorded: refuse loudly and
                // fail the run durably rather than inventing bytes.
                if let Ok(Some(record)) = self.repository.load_run(run_id).await {
                    let _ = self
                        .repository
                        .mark_run(run_id, &record.state, ImportState::Failed)
                        .await;
                }
                Err(ReceiptError::StagingEvidenceLost)
            }
            Err(error) => {
                if error.kind() == std::io::ErrorKind::NotFound {
                    return Err(ReceiptError::StagingEvidenceLost);
                }
                Err(ReceiptError::Storage(error))
            }
        }
    }

    /// Resumes an interrupted run from its persisted stage and surviving
    /// staging evidence.
    ///
    /// Returns `Ok(None)` when nothing may be manufactured: the run is
    /// terminal, or it sits at a stage later plan items own.
    ///
    /// # Errors
    ///
    /// Returns [`ReceiptError`] when staging evidence is lost or fails
    /// verification ([`ReceiptError::StagingEvidenceLost`]), or when storage
    /// or persistence refuse the work.
    pub async fn resume(&self, run_id: Uuid) -> Result<Option<ReceiptOutcome>, ReceiptError> {
        let Some(record) = self
            .repository
            .load_run(run_id)
            .await
            .map_err(ReceiptError::Repository)?
        else {
            return Err(ReceiptError::StagingEvidenceLost);
        };
        if record.state.is_terminal()
            || !matches!(record.state, ImportState::Received | ImportState::Hashed)
        {
            // Terminal runs are finished; stored-and-beyond stages belong to
            // later implementation items and are left untouched here.
            return Ok(None);
        }

        let staging_path = self.staging_path(run_id);
        let (sha256_hex, byte_length) = match record.state {
            ImportState::Received => {
                let evidence = hash_file(&staging_path).await.map_err(|error| {
                    if error.kind() == std::io::ErrorKind::NotFound {
                        ReceiptError::StagingEvidenceLost
                    } else {
                        ReceiptError::Storage(error)
                    }
                });
                let evidence = match evidence {
                    Ok(evidence) => evidence,
                    Err(error) => {
                        self.fail_run(run_id, &record.state).await;
                        return Err(error);
                    }
                };
                self.repository
                    .record_hash(run_id, evidence.0.clone(), evidence.1)
                    .await
                    .map_err(ReceiptError::Repository)?;
                evidence
            }
            ImportState::Hashed => {
                let expected_hex = record.sha256_hex.clone().ok_or_else(|| {
                    ReceiptError::Repository(RepositoryError::backend(std::io::Error::other(
                        "a hashed run carries no digest",
                    )))
                })?;
                let expected_length =
                    u64::try_from(record.byte_length.unwrap_or_default()).unwrap_or(u64::MAX);
                if let Err(error) = self
                    .verify_staged(run_id, &staging_path, &expected_hex, expected_length)
                    .await
                {
                    self.fail_run(run_id, &record.state).await;
                    return Err(error);
                }
                (expected_hex, expected_length)
            }
            _ => return Ok(None),
        };

        let context = HashedContext {
            run_id,
            tenant: record.account_external_ref,
            mode_spelling: record.acquisition_mode,
            media_type: record.media_type,
            sha256_hex,
            byte_length,
        };
        let outcome = self.settle_hashed(context, None, &staging_path).await;
        let _ = tokio::fs::remove_file(&staging_path).await;
        outcome.map(Some)
    }

    /// Best-effort startup recovery: resumes every run a previous process
    /// left at a resumable stage, logging each outcome by identity only.
    /// Returns how many runs were swept.
    pub async fn sweep_interrupted(&self) -> usize {
        let records = match self.repository.list_resumable().await {
            Ok(records) => records,
            Err(error) => {
                tracing::warn!(
                    chain = %error,
                    "the interrupted-run sweep could not read durable state"
                );
                return 0;
            }
        };
        let total = records.len();
        for record in records {
            match self.resume(record.id).await {
                Ok(Some(outcome)) => tracing::info!(
                    run_id = %record.id,
                    outcome = if matches!(outcome, ReceiptOutcome::Stored { .. }) {
                        "stored"
                    } else {
                        "duplicate"
                    },
                    "resumed an interrupted import run"
                ),
                Ok(None) => {}
                Err(error) => tracing::warn!(
                    run_id = %record.id,
                    chain = %error,
                    "an interrupted import run could not resume"
                ),
            }
        }
        total
    }
}

/// Everything `settle_hashed` needs once a run reaches `hashed`.
struct HashedContext {
    run_id: Uuid,
    tenant: String,
    mode_spelling: String,
    media_type: String,
    sha256_hex: String,
    byte_length: u64,
}

/// How streaming into staging failed.
enum StagingFailure {
    /// The cap was crossed; consumption stops within one chunk.
    Overgrown,
    /// The upload itself broke before delivering the body.
    Stream(StreamFailure),
    /// Local durable storage refused the work.
    Io(std::io::Error),
}

/// Streams chunks into an isolated staging file while hashing them.
///
/// Memory stays bounded by one chunk regardless of archive size. The file is
/// flushed and synced before the digest is trusted, so a crash after hashing
/// leaves verifiable evidence on disk.
async fn stream_to_staging<S, E>(
    staging_path: &Path,
    mut stream: S,
    max_archive_bytes: u64,
) -> Result<(String, u64), StagingFailure>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin + Send + 'static,
    E: std::error::Error + Send + Sync + 'static,
{
    use futures_util::StreamExt as _;
    use sha2::Digest as _;
    use tokio::io::AsyncWriteExt as _;

    let file = tokio::fs::File::create(staging_path)
        .await
        .map_err(StagingFailure::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .await
            .map_err(StagingFailure::Io)?;
    }
    let mut writer = tokio::io::BufWriter::new(file);
    let mut hasher = sha2::Sha256::new();
    let mut total: u64 = 0;

    while let Some(item) = stream.next().await {
        let chunk = item.map_err(|error| StagingFailure::Stream(StreamFailure(Box::new(error))))?;
        total = total.saturating_add(chunk.len() as u64);
        if total > max_archive_bytes {
            return Err(StagingFailure::Overgrown);
        }
        hasher.update(&chunk);
        writer.write_all(&chunk).await.map_err(StagingFailure::Io)?;
    }
    writer.flush().await.map_err(StagingFailure::Io)?;
    let file = writer.into_inner();
    file.sync_all().await.map_err(StagingFailure::Io)?;
    drop(file);

    Ok((hex::encode(hasher.finalize()), total))
}

/// Hashes a file from disk in bounded chunks.
///
/// # Errors
///
/// Propagates IO errors, including `NotFound` for absent evidence.
async fn hash_file(path: &Path) -> Result<(String, u64), std::io::Error> {
    use sha2::Digest as _;
    use tokio::io::AsyncReadExt as _;

    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = sha2::Sha256::new();
    let mut total: u64 = 0;
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        let Some(chunk) = buffer.get(..read) else {
            return Err(std::io::Error::other(
                "the reader reported a length outside the buffer",
            ));
        };
        hasher.update(chunk);
        total += read as u64;
    }
    Ok((hex::encode(hasher.finalize()), total))
}
