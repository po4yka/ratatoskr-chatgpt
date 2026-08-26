//! Bounded archive inspection and extraction.

use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use bytes::Bytes;
use ratatoskr_identifiers::BlobRef;
use tokio::io::AsyncReadExt as _;
use uuid::Uuid;

use crate::BlobStore;

/// The finite limits applied while reading one archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveLimits {
    /// Maximum entries allowed in the central directory.
    pub max_entries: u32,
    /// Maximum compressed bytes declared by all entries.
    pub max_compressed_bytes: u64,
    /// Maximum decompressed bytes for one entry.
    pub max_entry_bytes: u64,
    /// Maximum aggregate decompressed bytes.
    pub max_decompressed_bytes: u64,
    /// Maximum declared per-entry decompression ratio.
    pub max_compression_ratio: u64,
}

impl From<&crate::config::Limits> for ArchiveLimits {
    fn from(limits: &crate::config::Limits) -> Self {
        Self {
            max_entries: limits.max_archive_entries,
            max_compressed_bytes: limits.max_archive_bytes,
            max_entry_bytes: limits.max_archive_entry_bytes,
            max_decompressed_bytes: limits.max_archive_decompressed_bytes,
            max_compression_ratio: limits.max_archive_compression_ratio,
        }
    }
}

/// One inspected archive entry kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// A directory marker.
    Directory,
    /// A JSON-like structured record.
    Json,
    /// An HTML-like record that stays quarantined.
    Html,
    /// Binary media that stays quarantined.
    Media,
    /// Text that is not a recognized structured record.
    Text,
    /// Bytes without a recognized safe classification.
    Unknown,
}

/// Metadata observed without executing or rendering entry content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntry {
    /// Normalized archive-relative name.
    pub path: String,
    /// Coarse, non-semantic type signal from a bounded content prefix.
    pub kind: EntryKind,
    /// Declared compressed byte count.
    pub compressed_bytes: u64,
    /// Declared decompressed byte count.
    pub decompressed_bytes: u64,
}

/// A successful structural inventory supplied to later stages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveInventory {
    /// Entries in central-directory order.
    pub entries: Vec<ArchiveEntry>,
    /// Aggregate declared compressed byte count.
    pub compressed_bytes: u64,
    /// Aggregate declared decompressed byte count.
    pub decompressed_bytes: u64,
    /// Safe structure markers used by parser declarations.
    pub signals: BTreeSet<String>,
}

/// Why archive inspection or extraction refused its input.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ArchiveIntakeError {
    /// No supported archive reader accepted the raw bytes.
    #[error("the archive format is unsupported")]
    UnsupportedFormat,
    /// An archive path is not safe to represent.
    #[error("the archive contains an unsafe path")]
    UnsafePath,
    /// Two entries name one normalized path.
    #[error("the archive contains duplicate normalized names")]
    DuplicateName,
    /// An archive entry declares an unsupported special file.
    #[error("the archive contains a special file")]
    SpecialFile,
    /// Declared or observed bytes exceed a configured limit.
    #[error("the archive exceeds a configured limit")]
    LimitExceeded,
    /// The raw evidence, staging area, or `BlobStore` could not be used.
    #[error("the raw archive evidence is unavailable")]
    EvidenceUnavailable,
}

/// Read-only inspector for raw archive evidence.
#[derive(Debug, Clone)]
pub struct ArchiveInspector {
    blob: BlobStore,
    limits: ArchiveLimits,
}

/// Immutable source evidence for one extracted artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactProvenance {
    /// SHA-256 digest of the raw archive `BlobRef`.
    pub raw_archive_digest: String,
    /// Normalized archive-relative entry path.
    pub entry_path: String,
}

/// An extracted, immutable entry reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedArtifact {
    /// Content-addressed stored bytes.
    pub blob: BlobRef,
    /// Immutable evidence back to the received archive.
    pub provenance: ArtifactProvenance,
    /// True for HTML-like or media entries that must not be auto-trusted.
    pub quarantined: bool,
}

/// Bounded extractor that never uses archive names as output paths.
#[derive(Debug, Clone)]
pub struct ArchiveExtractor {
    blob: BlobStore,
    limits: ArchiveLimits,
    staging_root: PathBuf,
}

#[derive(Debug)]
struct StagedArtifact {
    entry: ArchiveEntry,
    path: PathBuf,
}

#[derive(Debug)]
struct StagedExtraction {
    root: PathBuf,
    artifacts: Vec<StagedArtifact>,
}

impl ArchiveExtractor {
    /// Creates an extractor with explicit hostile-input limits and an owned
    /// base directory for private staging.
    #[must_use]
    pub fn new(blob: BlobStore, limits: ArchiveLimits, staging_root: PathBuf) -> Self {
        Self {
            blob,
            limits,
            staging_root,
        }
    }

    /// Extracts an already-inspected archive into immutable references.
    ///
    /// # Errors
    ///
    /// Returns a typed refusal when evidence, ZIP structure, limits, staging,
    /// or blob storage fail verification. It removes only the UUID-named
    /// staging directory it created before returning.
    pub async fn extract(
        &self,
        raw: &BlobRef,
        inventory: &ArchiveInventory,
    ) -> Result<Vec<ExtractedArtifact>, ArchiveIntakeError> {
        let path = self
            .blob
            .verify(raw)
            .await
            .map_err(|_| ArchiveIntakeError::EvidenceUnavailable)?;
        let limits = self.limits.clone();
        let staging_root = self.staging_root.clone();
        let expected = inventory.clone();
        let staging = tokio::task::spawn_blocking(move || {
            extract_zip_to_staging(&path, &expected, &limits, &staging_root)
        })
        .await
        .map_err(|_| ArchiveIntakeError::EvidenceUnavailable)??;

        let mut artifacts = Vec::with_capacity(staging.artifacts.len());
        for staged in &staging.artifacts {
            let result = self.publish_staged_artifact(raw, staged).await;
            match result {
                Ok(artifact) => artifacts.push(artifact),
                Err(error) => {
                    remove_owned_staging(&staging.root).await?;
                    return Err(error);
                }
            }
        }
        remove_owned_staging(&staging.root).await?;
        Ok(artifacts)
    }

    async fn publish_staged_artifact(
        &self,
        raw: &BlobRef,
        staged: &StagedArtifact,
    ) -> Result<ExtractedArtifact, ArchiveIntakeError> {
        let file = tokio::fs::File::open(&staged.path)
            .await
            .map_err(|_| ArchiveIntakeError::EvidenceUnavailable)?;
        let stream = Box::pin(futures_util::stream::unfold(
            Some(file),
            |file| async move {
                let mut file = file?;
                let mut buffer = vec![0_u8; 64 * 1024];
                match file.read(&mut buffer).await {
                    Ok(0) => None,
                    Ok(read) => match buffer.get(..read) {
                        Some(chunk) => Some((Ok(Bytes::copy_from_slice(chunk)), Some(file))),
                        None => Some((Err(std::io::Error::other("invalid read length")), None)),
                    },
                    Err(error) => Some((Err(error), None)),
                }
            },
        ));
        let blob = self
            .blob
            .store(media_type(staged.entry.kind), stream)
            .await
            .map_err(|_| ArchiveIntakeError::EvidenceUnavailable)?;
        self.blob
            .verify(&blob)
            .await
            .map_err(|_| ArchiveIntakeError::EvidenceUnavailable)?;

        Ok(ExtractedArtifact {
            blob,
            provenance: ArtifactProvenance {
                raw_archive_digest: raw.digest.hex.as_str().to_owned(),
                entry_path: staged.entry.path.clone(),
            },
            quarantined: matches!(staged.entry.kind, EntryKind::Html | EntryKind::Media),
        })
    }
}

impl ArchiveInspector {
    /// Creates an inspector with explicit hostile-input limits.
    #[must_use]
    pub fn new(blob: BlobStore, limits: ArchiveLimits) -> Self {
        Self { blob, limits }
    }

    /// Inspects verified raw evidence without executing or rendering entries.
    ///
    /// # Errors
    ///
    /// Returns a typed refusal when evidence, ZIP structure, or limits are invalid.
    pub async fn inspect(&self, raw: &BlobRef) -> Result<ArchiveInventory, ArchiveIntakeError> {
        let path = self
            .blob
            .verify(raw)
            .await
            .map_err(|_| ArchiveIntakeError::EvidenceUnavailable)?;
        let limits = self.limits.clone();
        tokio::task::spawn_blocking(move || inspect_zip(&path, &limits))
            .await
            .map_err(|_| ArchiveIntakeError::EvidenceUnavailable)?
    }
}

fn inspect_zip(
    path: &Path,
    limits: &ArchiveLimits,
) -> Result<ArchiveInventory, ArchiveIntakeError> {
    let file = File::open(path).map_err(|_| ArchiveIntakeError::EvidenceUnavailable)?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|_| ArchiveIntakeError::UnsupportedFormat)?;
    if archive.len() > usize::try_from(limits.max_entries).unwrap_or(usize::MAX) {
        return Err(ArchiveIntakeError::LimitExceeded);
    }

    let mut names = BTreeSet::new();
    let mut entries = Vec::with_capacity(archive.len());
    let mut compressed_bytes = 0_u64;
    let mut decompressed_bytes = 0_u64;
    let mut signals = BTreeSet::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|_| ArchiveIntakeError::UnsupportedFormat)?;
        let normalized = normalize_path(entry.name())?;
        if !names.insert(normalized.clone()) {
            return Err(ArchiveIntakeError::DuplicateName);
        }
        reject_special_file(&entry)?;

        let compressed = entry.compressed_size();
        let decompressed = entry.size();
        checked_metadata_totals(
            limits,
            compressed,
            decompressed,
            &mut compressed_bytes,
            &mut decompressed_bytes,
        )?;
        let kind = if entry.is_dir() {
            EntryKind::Directory
        } else {
            EntryKind::Unknown
        };
        if !entry.is_dir() {
            signals.insert(normalized.rsplit('/').next().unwrap_or_default().to_owned());
        }
        entries.push(ArchiveEntry {
            path: normalized,
            kind,
            compressed_bytes: compressed,
            decompressed_bytes: decompressed,
        });
    }
    for (index, inspected) in entries.iter_mut().enumerate() {
        if inspected.kind == EntryKind::Directory {
            continue;
        }
        let mut entry = archive
            .by_index(index)
            .map_err(|_| ArchiveIntakeError::UnsupportedFormat)?;
        inspected.kind = classify_prefix(&mut entry)?;
    }
    Ok(ArchiveInventory {
        entries,
        compressed_bytes,
        decompressed_bytes,
        signals,
    })
}

fn extract_zip_to_staging(
    path: &Path,
    expected: &ArchiveInventory,
    limits: &ArchiveLimits,
    staging_root: &Path,
) -> Result<StagedExtraction, ArchiveIntakeError> {
    let inspected = inspect_zip(path, limits)?;
    if &inspected != expected {
        return Err(ArchiveIntakeError::UnsupportedFormat);
    }
    std::fs::create_dir_all(staging_root).map_err(|_| ArchiveIntakeError::EvidenceUnavailable)?;
    restrict_directory(staging_root)?;
    let root = staging_root.join(format!("archive-{}", Uuid::now_v7()));
    std::fs::create_dir(&root).map_err(|_| ArchiveIntakeError::EvidenceUnavailable)?;
    restrict_directory(&root)?;

    let outcome = extract_entries(path, &inspected.entries, limits, &root);
    if outcome.is_err() {
        let _ = std::fs::remove_dir_all(&root);
    }
    outcome.map(|artifacts| StagedExtraction { root, artifacts })
}

fn extract_entries(
    path: &Path,
    expected: &[ArchiveEntry],
    limits: &ArchiveLimits,
    root: &Path,
) -> Result<Vec<StagedArtifact>, ArchiveIntakeError> {
    let file = File::open(path).map_err(|_| ArchiveIntakeError::EvidenceUnavailable)?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|_| ArchiveIntakeError::UnsupportedFormat)?;
    let mut total = 0_u64;
    let mut artifacts = Vec::new();
    for (index, observed) in expected.iter().enumerate() {
        if observed.kind == EntryKind::Directory {
            continue;
        }
        let mut entry = archive
            .by_index(index)
            .map_err(|_| ArchiveIntakeError::UnsupportedFormat)?;
        if normalize_path(entry.name())? != observed.path {
            return Err(ArchiveIntakeError::UnsafePath);
        }

        let staged_path = root.join(format!("entry-{index}.part"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged_path)
            .map_err(|_| ArchiveIntakeError::EvidenceUnavailable)?;
        restrict_file(&staged_path)?;
        let mut writer = std::io::BufWriter::new(file);
        let mut entry_total = 0_u64;
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            let read = entry
                .read(&mut buffer)
                .map_err(|_| ArchiveIntakeError::UnsupportedFormat)?;
            if read == 0 {
                break;
            }
            let bytes = u64::try_from(read).map_err(|_| ArchiveIntakeError::LimitExceeded)?;
            entry_total = entry_total
                .checked_add(bytes)
                .ok_or(ArchiveIntakeError::LimitExceeded)?;
            total = total
                .checked_add(bytes)
                .ok_or(ArchiveIntakeError::LimitExceeded)?;
            if entry_total > limits.max_entry_bytes || total > limits.max_decompressed_bytes {
                return Err(ArchiveIntakeError::LimitExceeded);
            }
            let chunk = buffer
                .get(..read)
                .ok_or(ArchiveIntakeError::EvidenceUnavailable)?;
            writer
                .write_all(chunk)
                .map_err(|_| ArchiveIntakeError::EvidenceUnavailable)?;
        }
        writer
            .flush()
            .map_err(|_| ArchiveIntakeError::EvidenceUnavailable)?;
        drop(writer);
        if entry_total != observed.decompressed_bytes {
            return Err(ArchiveIntakeError::UnsupportedFormat);
        }
        artifacts.push(StagedArtifact {
            entry: observed.clone(),
            path: staged_path,
        });
    }
    Ok(artifacts)
}

fn reject_special_file(entry: &zip::read::ZipFile<'_, File>) -> Result<(), ArchiveIntakeError> {
    if let Some(mode) = entry.unix_mode() {
        let kind = mode & 0o170_000;
        if kind != 0 && kind != 0o100_000 && kind != 0o040_000 {
            return Err(ArchiveIntakeError::SpecialFile);
        }
    }
    Ok(())
}

fn checked_metadata_totals(
    limits: &ArchiveLimits,
    compressed: u64,
    decompressed: u64,
    compressed_total: &mut u64,
    decompressed_total: &mut u64,
) -> Result<(), ArchiveIntakeError> {
    let next_compressed = compressed_total
        .checked_add(compressed)
        .ok_or(ArchiveIntakeError::LimitExceeded)?;
    let next_decompressed = decompressed_total
        .checked_add(decompressed)
        .ok_or(ArchiveIntakeError::LimitExceeded)?;
    if compressed > limits.max_compressed_bytes
        || next_compressed > limits.max_compressed_bytes
        || decompressed > limits.max_entry_bytes
        || next_decompressed > limits.max_decompressed_bytes
        || (compressed == 0 && decompressed > 0)
        || (compressed > 0
            && decompressed > compressed.saturating_mul(limits.max_compression_ratio))
    {
        return Err(ArchiveIntakeError::LimitExceeded);
    }
    *compressed_total = next_compressed;
    *decompressed_total = next_decompressed;
    Ok(())
}

fn normalize_path(name: &str) -> Result<String, ArchiveIntakeError> {
    if name.is_empty()
        || name.starts_with('/')
        || name.starts_with('\\')
        || name.split(['/', '\\']).any(|part| part == "..")
    {
        return Err(ArchiveIntakeError::UnsafePath);
    }
    let parts: Vec<_> = name
        .split(['/', '\\'])
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    if parts.is_empty() || parts.iter().any(|part| part.contains(':')) {
        return Err(ArchiveIntakeError::UnsafePath);
    }
    Ok(parts.join("/"))
}

fn classify_prefix(
    entry: &mut zip::read::ZipFile<'_, File>,
) -> Result<EntryKind, ArchiveIntakeError> {
    let mut prefix = [0_u8; 512];
    let read = entry
        .read(&mut prefix)
        .map_err(|_| ArchiveIntakeError::UnsupportedFormat)?;
    let prefix = prefix
        .get(..read)
        .ok_or(ArchiveIntakeError::UnsupportedFormat)?;
    Ok(classify_bytes(prefix))
}

fn classify_bytes(bytes: &[u8]) -> EntryKind {
    let trimmed = trim_ascii_whitespace(bytes);
    if trimmed.starts_with(b"{") || trimmed.starts_with(b"[") {
        EntryKind::Json
    } else if starts_html(trimmed) {
        EntryKind::Html
    } else if is_media(trimmed) {
        EntryKind::Media
    } else if std::str::from_utf8(trimmed).is_ok() {
        EntryKind::Text
    } else {
        EntryKind::Unknown
    }
}

fn trim_ascii_whitespace(bytes: &[u8]) -> &[u8] {
    let first = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    bytes.get(first..).unwrap_or_default()
}

fn starts_html(bytes: &[u8]) -> bool {
    let prefix = bytes
        .iter()
        .take(32)
        .map(u8::to_ascii_lowercase)
        .collect::<Vec<_>>();
    prefix.starts_with(b"<html") || prefix.starts_with(b"<!doctype html")
}

fn is_media(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(b"\xff\xd8\xff")
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
        || bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP")
}

fn media_type(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Json => "application/json",
        EntryKind::Html => "text/html",
        EntryKind::Text => "text/plain",
        EntryKind::Directory | EntryKind::Media | EntryKind::Unknown => "application/octet-stream",
    }
}

fn restrict_directory(path: &Path) -> Result<(), ArchiveIntakeError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| ArchiveIntakeError::EvidenceUnavailable)?;
    }
    Ok(())
}

fn restrict_file(path: &Path) -> Result<(), ArchiveIntakeError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|_| ArchiveIntakeError::EvidenceUnavailable)?;
    }
    Ok(())
}

async fn remove_owned_staging(path: &Path) -> Result<(), ArchiveIntakeError> {
    tokio::fs::remove_dir_all(path)
        .await
        .map_err(|_| ArchiveIntakeError::EvidenceUnavailable)
}
