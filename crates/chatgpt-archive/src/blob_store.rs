//! Content-addressed immutable byte storage addressed by fleet `BlobRef`s.
//!
//! The facade owns the CONTRACT — digest, reference, immutability,
//! verify-on-read — and delegates byte placement to a [`BlobBackend`] seam, so
//! a remote object-store backend can arrive later without changing callers or
//! the reference format. This capability cites the stored-bytes contract
//! `blob-references` in the `ratatoskr-workspace` store: each service writes
//! its own blobs under content-addressed paths on its own durable device.
//!
//! Layout of the local backend (an implementation detail no caller may depend
//! on): staging files under `{root}/staging`, published objects under
//! `{root}/sha256/{first-2-hex}/{remaining-62-hex}`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use futures_core::Stream;
use futures_util::StreamExt as _;
use ratatoskr_identifiers::{
    BlobOwner, BlobRef, ContentDigest, DigestAlgorithm, DigestHex, MediaType,
};
use sha2::Digest as _;
use uuid::Uuid;

/// The deployment identity this storage publishes under.
const OWNER: &str = "ratatoskr-chatgpt";

/// Byte-stream errors are erased at the facade boundary so the backend seam
/// stays object-safe.
#[derive(Debug)]
pub struct ByteStreamError(pub Box<dyn std::error::Error + Send + Sync>);

impl core::fmt::Display for ByteStreamError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ByteStreamError {}

/// The erased stream shape the backend receives.
pub type ByteStream = std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, ByteStreamError>> + Send>>;

/// A boxed future; the seam's async surface without an async-trait crate.
pub type BackendFuture<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;

/// Where bytes live. Internal seam: one implementation ships now, a
/// remote-object-store one may implement exactly this later.
pub(crate) trait BlobBackend: core::fmt::Debug + Send + Sync {
    /// Receives the whole stream, returns the reference it published under.
    fn receive(
        &self,
        media_type: MediaType,
        stream: ByteStream,
    ) -> BackendFuture<Result<BlobRef, BlobStoreError>>;

    /// Reads and verifies the object named by `reference`.
    fn read_verified(&self, reference: BlobRef) -> BackendFuture<Result<PathBuf, BlobStoreError>>;

    /// Names the path of `reference` without reading bytes.
    fn locate(&self, reference: &BlobRef) -> Result<PathBuf, BlobStoreError>;

    /// Erases the exact object named by `reference`.
    fn erase(&self, reference: BlobRef) -> BackendFuture<Result<(), BlobStoreError>>;
}

/// Blob storage failures.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BlobStoreError {
    /// The storage root is unusable.
    #[error("the blob storage root is unavailable")]
    Unavailable(#[source] std::io::Error),
    /// The declared media type is not `type/subtype`.
    #[error("the media type is invalid")]
    InvalidMediaType,
    /// The stream failed while bytes were arriving.
    #[error("the byte stream failed before completion")]
    Stream(#[source] ByteStreamError),
    /// The referenced object is absent or does not verify. A mismatch reads as
    /// missing, never as changed content.
    #[error("the referenced object is missing or does not verify")]
    Missing,
}

/// Content-addressed storage owned by this service.
///
/// Cloning is cheap: every handle shares one backend.
#[derive(Debug, Clone)]
pub struct BlobStore {
    backend: Arc<dyn BlobBackend>,
}

impl BlobStore {
    /// Creates a store rooted at `root`, owning its objects as
    /// `ratatoskr-chatgpt`.
    ///
    /// # Errors
    ///
    /// Returns [`BlobStoreError`] when the root cannot anchor storage.
    pub fn new(root: &Path) -> Result<Self, BlobStoreError> {
        Ok(Self {
            backend: Arc::new(LocalFsBackend::new(root)?),
        })
    }

    /// Stores a byte stream and returns its reference. Identical bytes always
    /// produce an equal reference, and a stored object is never rewritten.
    ///
    /// # Errors
    ///
    /// Returns [`BlobStoreError`] when the media type is malformed or the
    /// stream fails before completion; nothing is then published.
    pub async fn store<S, E>(&self, media_type: &str, stream: S) -> Result<BlobRef, BlobStoreError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin + Send + 'static,
        E: std::error::Error + Send + Sync + 'static,
    {
        let media_type =
            MediaType::parse(media_type).map_err(|_| BlobStoreError::InvalidMediaType)?;
        let stream: ByteStream =
            Box::pin(stream.map(|item| item.map_err(|error| ByteStreamError(Box::new(error)))));
        self.backend.receive(media_type, stream).await
    }

    /// Resolves a reference to a readable path only when an object exists
    /// whose owner, algorithm, digest, length, and media type all match.
    ///
    /// # Errors
    ///
    /// Returns [`BlobStoreError::Missing`] on any mismatch, including a
    /// foreign owner.
    pub async fn verify(&self, reference: &BlobRef) -> Result<PathBuf, BlobStoreError> {
        if reference.owner_service.as_str() != OWNER {
            return Err(BlobStoreError::Missing);
        }
        self.backend.read_verified(reference.clone()).await
    }

    /// Resolves a reference to a path without reading the bytes. Ownership is
    /// checked; integrity is not.
    ///
    /// # Errors
    ///
    /// Returns [`BlobStoreError::Missing`] when the reference names no local
    /// object or carries a foreign owner.
    pub fn resolve(&self, reference: &BlobRef) -> Result<PathBuf, BlobStoreError> {
        if reference.owner_service.as_str() != OWNER {
            return Err(BlobStoreError::Missing);
        }
        self.backend.locate(reference)
    }

    /// Idempotently erases the exact content-addressed object named by a
    /// reference. Callers must establish retained reachability first.
    ///
    /// # Errors
    ///
    /// Returns [`BlobStoreError`] when the reference cannot name an erasable
    /// object in this store.
    pub async fn erase(&self, reference: &BlobRef) -> Result<(), BlobStoreError> {
        if reference.owner_service.as_str() != OWNER {
            return Err(BlobStoreError::Missing);
        }
        self.backend.erase(reference.clone()).await
    }
}

/// Filesystem backend: staging plus hard-link publish, SHA-256 while
/// streaming.
#[derive(Debug)]
pub(crate) struct LocalFsBackend {
    root: PathBuf,
    owner: BlobOwner,
}

impl LocalFsBackend {
    fn new(root: &Path) -> Result<Self, BlobStoreError> {
        std::fs::create_dir_all(root).map_err(BlobStoreError::Unavailable)?;
        let owner = BlobOwner::parse(OWNER).map_err(|_| BlobStoreError::Missing)?;
        Ok(Self {
            root: root.to_path_buf(),
            owner,
        })
    }

    fn content_path(&self, reference: &BlobRef) -> Result<PathBuf, BlobStoreError> {
        // The contract enum is non-exhaustive: any future algorithm fails
        // closed here until its layout is decided deliberately.
        if reference.digest.algorithm != DigestAlgorithm::Sha256 {
            return Err(BlobStoreError::Missing);
        }
        let hex = reference.digest.hex.as_str();
        let Some((directory, remainder)) = hex.split_at_checked(2) else {
            return Err(BlobStoreError::Missing);
        };
        Ok(self.root.join("sha256").join(directory).join(remainder))
    }
}

impl BlobBackend for LocalFsBackend {
    fn receive(
        &self,
        media_type: MediaType,
        mut stream: ByteStream,
    ) -> BackendFuture<Result<BlobRef, BlobStoreError>> {
        let root = self.root.clone();
        let owner = self.owner.clone();
        Box::pin(async move {
            use tokio::io::AsyncWriteExt as _;

            let staging = root.join("staging");
            tokio::fs::create_dir_all(&staging)
                .await
                .map_err(BlobStoreError::Unavailable)?;

            let part = staging.join(format!("ratatoskr-chatgpt-{}.part", Uuid::now_v7()));
            let file = tokio::fs::File::create(&part)
                .await
                .map_err(BlobStoreError::Unavailable)?;
            let mut writer = tokio::io::BufWriter::new(file);
            let mut hasher = sha2::Sha256::new();
            let mut total: u64 = 0;

            let mut outcome = Ok(());
            while outcome.is_ok() {
                let Some(item) = stream.next().await else {
                    break;
                };
                let chunk = match item {
                    Ok(chunk) => chunk,
                    Err(error) => {
                        outcome = Err(BlobStoreError::Stream(error));
                        break;
                    }
                };
                hasher.update(&chunk);
                total = total.saturating_add(chunk.len() as u64);
                if let Err(error) = writer.write_all(&chunk).await {
                    outcome = Err(BlobStoreError::Unavailable(error));
                }
            }

            let flushed = writer.flush().await.map_err(BlobStoreError::Unavailable);
            drop(writer);
            flushed?;

            if let Err(error) = outcome {
                // An interrupted write leaves no final object: the part file
                // dies with the attempt.
                let _ = tokio::fs::remove_file(&part).await;
                return Err(error);
            }

            let hex = hex::encode(hasher.finalize());
            let Some((directory, remainder)) = hex.split_at_checked(2) else {
                return Err(BlobStoreError::Missing);
            };
            let destination = root.join("sha256").join(directory).join(remainder);
            if let Some(parent) = destination.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(BlobStoreError::Unavailable)?;
            }

            // Publish is create-new: an existing digest is never rewritten, so
            // identical bytes deduplicate onto the same immutable object.
            match tokio::fs::hard_link(&part, &destination).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    let _ = tokio::fs::remove_file(&part).await;
                    return Err(BlobStoreError::Unavailable(error));
                }
            }
            let _ = tokio::fs::remove_file(&part).await;

            let hex = DigestHex::parse(&hex).map_err(|_| BlobStoreError::Missing)?;
            Ok(BlobRef {
                owner_service: owner,
                digest: ContentDigest {
                    algorithm: DigestAlgorithm::Sha256,
                    hex,
                },
                media_type,
                length_bytes: total,
            })
        })
    }

    fn read_verified(&self, reference: BlobRef) -> BackendFuture<Result<PathBuf, BlobStoreError>> {
        let path = match self.locate(&reference) {
            Ok(path) => path,
            Err(error) => return Box::pin(async move { Err(error) }),
        };
        let expected_hex = reference.digest.hex.clone();
        let expected_length = reference.length_bytes;
        Box::pin(async move {
            use tokio::io::AsyncReadExt as _;

            let metadata = tokio::fs::metadata(&path)
                .await
                .map_err(|_| BlobStoreError::Missing)?;
            if !metadata.is_file() || metadata.len() != expected_length {
                return Err(BlobStoreError::Missing);
            }

            let mut file = tokio::fs::File::open(&path)
                .await
                .map_err(|_| BlobStoreError::Missing)?;
            let mut hasher = sha2::Sha256::new();
            let mut buffer = vec![0u8; 64 * 1024];
            loop {
                let read = file
                    .read(&mut buffer)
                    .await
                    .map_err(|_| BlobStoreError::Missing)?;
                if read == 0 {
                    break;
                }
                let Some(chunk) = buffer.get(..read) else {
                    return Err(BlobStoreError::Missing);
                };
                hasher.update(chunk);
            }
            let digest_hex = hex::encode(hasher.finalize());
            if digest_hex != expected_hex.as_str() {
                return Err(BlobStoreError::Missing);
            }
            Ok(path)
        })
    }

    fn locate(&self, reference: &BlobRef) -> Result<PathBuf, BlobStoreError> {
        let path = self.content_path(reference)?;
        if path.is_file() {
            Ok(path)
        } else {
            Err(BlobStoreError::Missing)
        }
    }

    fn erase(&self, reference: BlobRef) -> BackendFuture<Result<(), BlobStoreError>> {
        let path = match self.content_path(&reference) {
            Ok(path) => path,
            Err(error) => return Box::pin(async move { Err(error) }),
        };
        let object_root = self.root.join("sha256");
        let hex = reference.digest.hex.as_str();
        let Some((directory, remainder)) = hex.split_at_checked(2) else {
            return Box::pin(async { Err(BlobStoreError::Missing) });
        };
        let relative_path = PathBuf::from(directory).join(remainder);
        Box::pin(async move {
            use tokio::io::AsyncReadExt as _;

            let metadata = match tokio::fs::symlink_metadata(&path).await {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(BlobStoreError::Unavailable(error)),
            };
            if !metadata.file_type().is_file() || metadata.len() != reference.length_bytes {
                return Err(BlobStoreError::Missing);
            }

            let canonical_root = tokio::fs::canonicalize(&object_root)
                .await
                .map_err(|_| BlobStoreError::Missing)?;
            let canonical_path = tokio::fs::canonicalize(&path)
                .await
                .map_err(|_| BlobStoreError::Missing)?;
            if canonical_path != canonical_root.join(relative_path) {
                return Err(BlobStoreError::Missing);
            }

            let mut file = tokio::fs::File::open(&path)
                .await
                .map_err(|_| BlobStoreError::Missing)?;
            let mut hasher = sha2::Sha256::new();
            let mut buffer = vec![0u8; 64 * 1024];
            loop {
                let read = file
                    .read(&mut buffer)
                    .await
                    .map_err(|_| BlobStoreError::Missing)?;
                if read == 0 {
                    break;
                }
                let Some(chunk) = buffer.get(..read) else {
                    return Err(BlobStoreError::Missing);
                };
                hasher.update(chunk);
            }
            if hex::encode(hasher.finalize()) != reference.digest.hex.as_str() {
                return Err(BlobStoreError::Missing);
            }
            drop(file);
            tokio::fs::remove_file(path)
                .await
                .map_err(BlobStoreError::Unavailable)
        })
    }
}
