//! The one lock protocol for blob registration and blob erasure.
//!
//! The blob store is content-addressed and shared across tenants and exports, so two actors
//! can race on one digest: an eraser that re-checks "is this digest still referenced" and then
//! erases, and an import that stores identical bytes (a no-op because they exist) and registers
//! a reference afterwards (XR-021 CONTRACTS.md R2-07).
//!
//! The lock key is `pg_advisory_xact_lock(hashtextextended('blob:' || <digest_hex>, 0))` in the
//! service's own database. The order is fixed: the tenant lock first, then digest locks in
//! ascending digest order.
//!
//! - The eraser takes the digest lock, re-checks every reference column, and either keeps the
//!   blob or erases it and records that, committing while it still holds the lock.
//! - A registrar takes the digest lock before it inserts the first row that references the
//!   digest and then proves the blob is present. If an eraser won, it re-stores the bytes it
//!   still holds, and otherwise fails with [`RegistrationError::Unavailable`], a retryable
//!   class, and registers nothing.
//!
//! The invariant: no committed row references a missing blob.

use std::path::Path;

use bytes::Bytes;
use futures_util::stream;
use ratatoskr_identifiers::BlobRef;
use sqlx::{Postgres, Transaction};
use tokio::io::AsyncReadExt as _;

use crate::BlobStore;

/// Read size when re-storing staged bytes, matching the receipt's own staging reads.
const RESTORE_CHUNK_BYTES: usize = 64 * 1024;

/// Takes the digest lock for the rest of the transaction.
///
/// # Errors
///
/// Returns the database error when the lock cannot be taken.
pub(crate) async fn blob_lock(
    transaction: &mut Transaction<'_, Postgres>,
    digest_hex: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('blob:' || $1::text, 0))")
        .bind(digest_hex)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

/// The bytes a registrar still holds for a blob it is about to reference.
#[derive(Debug, Clone, Copy)]
pub(crate) enum HeldBytes<'a> {
    /// A staging file that outlives the registering transaction.
    Staged(&'a Path),
    /// Bytes already in memory.
    Memory(&'a Bytes),
}

/// One blob a transaction is about to reference.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Registration<'a> {
    pub(crate) reference: &'a BlobRef,
    pub(crate) held: HeldBytes<'a>,
}

/// Why a blob reference could not be registered.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RegistrationError {
    /// The digest lock could not be taken.
    #[error("the blob lock could not be taken")]
    Database(#[from] sqlx::Error),
    /// The blob is gone and the registrar cannot restore it. Retryable: the next attempt stores
    /// the bytes again from the source.
    #[error("the blob is missing and could not be restored")]
    Unavailable,
}

/// Locks every digest in ascending order, then proves each blob present, restoring a missing
/// one from the bytes the caller still holds.
///
/// The caller inserts the referencing rows in the same transaction afterwards, so the lock is
/// held from the check to the commit.
///
/// # Errors
///
/// Returns [`RegistrationError::Unavailable`] when a blob is missing and cannot be restored;
/// nothing is registered then.
pub(crate) async fn register_blobs(
    transaction: &mut Transaction<'_, Postgres>,
    store: &BlobStore,
    registrations: &[Registration<'_>],
) -> Result<(), RegistrationError> {
    let mut digests: Vec<&str> = registrations
        .iter()
        .map(|registration| registration.reference.digest.hex.as_str())
        .collect();
    digests.sort_unstable();
    digests.dedup();
    for digest in digests {
        blob_lock(transaction, digest).await?;
    }
    for registration in registrations {
        if store.verify(registration.reference).await.is_ok() {
            continue;
        }
        restore(store, registration).await?;
    }
    Ok(())
}

async fn restore(
    store: &BlobStore,
    registration: &Registration<'_>,
) -> Result<(), RegistrationError> {
    let reference = registration.reference;
    let media_type = reference.media_type.as_str();
    let stored = match registration.held {
        HeldBytes::Memory(bytes) => store
            .store(
                media_type,
                stream::iter([Ok::<Bytes, std::io::Error>(bytes.clone())]),
            )
            .await
            .map_err(|_| RegistrationError::Unavailable)?,
        HeldBytes::Staged(path) => {
            let file = tokio::fs::File::open(path)
                .await
                .map_err(|_| RegistrationError::Unavailable)?;
            let chunks = stream::unfold(
                (file, vec![0_u8; RESTORE_CHUNK_BYTES]),
                |(mut file, mut buffer)| async move {
                    match file.read(&mut buffer).await {
                        Ok(0) => None,
                        Ok(read) => buffer
                            .get(..read)
                            .map(|filled| {
                                Ok::<Bytes, std::io::Error>(Bytes::copy_from_slice(filled))
                            })
                            .map(|chunk| (chunk, (file, buffer))),
                        Err(error) => Some((Err(error), (file, buffer))),
                    }
                },
            );
            store
                .store(media_type, Box::pin(chunks))
                .await
                .map_err(|_| RegistrationError::Unavailable)?
        }
    };
    // Bytes that no longer hash to the referenced digest are not the blob: a mismatch reads as
    // unavailable, never as the reference quietly changing.
    if stored.digest == reference.digest && stored.length_bytes == reference.length_bytes {
        Ok(())
    } else {
        Err(RegistrationError::Unavailable)
    }
}
