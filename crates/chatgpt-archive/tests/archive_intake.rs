//! Hostile ZIP acceptance tests for archive intake.
#![allow(clippy::expect_used, reason = "test setup must fail loudly")]

use std::io::Write as _;

use bytes::Bytes;
use ratatoskr_chatgpt_archive::{
    ArchiveExtractor, ArchiveInspector, ArchiveIntakeError, ArchiveLimits, BlobStore, EntryKind,
};
use zip::write::SimpleFileOptions;

fn limits() -> ArchiveLimits {
    ArchiveLimits {
        max_entries: 16,
        max_compressed_bytes: 4096,
        max_entry_bytes: 1024,
        max_decompressed_bytes: 4096,
        max_compression_ratio: 8,
    }
}

fn zip(entries: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (path, bytes) in entries {
        let options = if deflate {
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)
        } else {
            SimpleFileOptions::default()
        };
        writer
            .start_file(*path, options)
            .expect("synthetic ZIP entry");
        writer.write_all(bytes).expect("synthetic ZIP payload");
    }
    writer.finish().expect("synthetic ZIP finish").into_inner()
}

async fn inspect(
    bytes: Vec<u8>,
) -> Result<ratatoskr_chatgpt_archive::ArchiveInventory, ArchiveIntakeError> {
    let root = tempfile::tempdir().expect("temporary blob root");
    let store = BlobStore::new(root.path()).expect("blob store");
    let raw = store
        .store(
            "application/zip",
            futures_util::stream::iter(vec![Ok::<Bytes, std::io::Error>(Bytes::from(bytes))]),
        )
        .await
        .expect("raw evidence");
    ArchiveInspector::new(store, limits()).inspect(&raw).await
}

#[tokio::test]
async fn inspector_lists_structure_with_bounded_safe_type_detection() {
    let inventory = inspect(zip(
        &[
            ("conversations.json", b"{}"),
            ("chat.html", b"<html>"),
            ("image.png", b"\x89PNG\r\n\x1a\n"),
        ],
        false,
    ))
    .await
    .expect("safe ZIP must inspect");
    assert_eq!(inventory.entries[0].kind, EntryKind::Json);
    assert_eq!(inventory.entries[1].kind, EntryKind::Html);
    assert_eq!(inventory.entries[2].kind, EntryKind::Media);
}

#[tokio::test]
async fn type_detection_does_not_trust_a_media_extension() {
    let inventory = inspect(zip(&[("disguised.png", b"plain text")], false))
        .await
        .expect("safe ZIP must inspect");

    assert_eq!(inventory.entries[0].kind, EntryKind::Text);
}

#[tokio::test]
async fn zip_slip_is_rejected_before_extraction() {
    assert!(matches!(
        inspect(zip(&[("/outside.json", b"{}")], false)).await,
        Err(ArchiveIntakeError::UnsafePath)
    ));
}

#[tokio::test]
async fn traversal_is_rejected_before_extraction() {
    assert!(matches!(
        inspect(zip(&[("../outside.json", b"{}")], false)).await,
        Err(ArchiveIntakeError::UnsafePath)
    ));
}

#[tokio::test]
async fn duplicate_normalized_names_are_rejected() {
    assert!(matches!(
        inspect(zip(&[("a/b.json", b"{}"), ("a\\b.json", b"{}")], false)).await,
        Err(ArchiveIntakeError::DuplicateName)
    ));
}

#[tokio::test]
async fn declared_bomb_is_rejected_before_decompression() {
    assert!(matches!(
        inspect(zip(&[("large.json", &[b'x'; 512])], true)).await,
        Err(ArchiveIntakeError::LimitExceeded)
    ));
}

#[tokio::test]
async fn extracted_artifact_has_verified_blobref_and_raw_digest_provenance() {
    let root = tempfile::tempdir().expect("temporary blob root");
    let store = BlobStore::new(root.path()).expect("blob store");
    let raw = store
        .store(
            "application/zip",
            futures_util::stream::iter(vec![Ok::<Bytes, std::io::Error>(Bytes::from(zip(
                &[("conversations.json", b"{}")],
                false,
            )))]),
        )
        .await
        .expect("raw ZIP");
    let inventory = ArchiveInspector::new(store.clone(), limits())
        .inspect(&raw)
        .await
        .expect("inspection");
    let artifacts = ArchiveExtractor::new(store.clone(), limits(), root.path().join("extracting"))
        .extract(&raw, &inventory)
        .await
        .expect("extraction");
    assert_eq!(artifacts.len(), 1);
    assert_eq!(
        artifacts[0].provenance.raw_archive_digest,
        raw.digest.hex.as_str()
    );
    store
        .verify(&artifacts[0].blob)
        .await
        .expect("artifact verifies");
}

#[tokio::test]
async fn media_is_quarantined_reference() {
    let root = tempfile::tempdir().expect("temporary blob root");
    let store = BlobStore::new(root.path()).expect("blob store");
    let raw = store
        .store(
            "application/zip",
            futures_util::stream::iter(vec![Ok::<Bytes, std::io::Error>(Bytes::from(zip(
                &[("image.png", b"\x89PNG\r\n\x1a\n")],
                false,
            )))]),
        )
        .await
        .expect("raw ZIP");
    let inventory = ArchiveInspector::new(store.clone(), limits())
        .inspect(&raw)
        .await
        .expect("inspection");
    let artifacts = ArchiveExtractor::new(store, limits(), root.path().join("extracting"))
        .extract(&raw, &inventory)
        .await
        .expect("extraction");
    assert!(artifacts[0].quarantined);
}

#[tokio::test]
async fn extractor_cleans_only_its_owned_staging_directory() {
    let root = tempfile::tempdir().expect("temporary blob root");
    let staging_root = root.path().join("extracting");
    let store = BlobStore::new(root.path()).expect("blob store");
    let raw = store
        .store(
            "application/zip",
            futures_util::stream::iter(vec![Ok::<Bytes, std::io::Error>(Bytes::from(zip(
                &[("conversations.json", b"{}")],
                false,
            )))]),
        )
        .await
        .expect("raw ZIP");
    let inventory = ArchiveInspector::new(store.clone(), limits())
        .inspect(&raw)
        .await
        .expect("inspection");

    ArchiveExtractor::new(store, limits(), staging_root.clone())
        .extract(&raw, &inventory)
        .await
        .expect("extraction");

    assert!(staging_root.is_dir());
    assert!(
        std::fs::read_dir(staging_root)
            .expect("owned staging root is readable")
            .next()
            .is_none(),
        "per-run staging directories must be removed after publication"
    );
}
