//! Contract tests for the content-addressed `BlobStore` adapter.
//!
//! Every test targets the facade only; the on-disk layout is an implementation
//! detail no test may depend on.

use bytes::Bytes;
use futures_util::stream;
use ratatoskr_chatgpt_archive::blob_store::BlobStore;

/// A finite byte stream from an in-memory buffer.
fn stream_of(
    bytes: &[u8],
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Unpin {
    Box::pin(stream::iter(
        bytes
            .chunks(7)
            .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
            .collect::<Vec<_>>(),
    ))
}

const MEDIA_TYPE: &str = "application/octet-stream";

/// Identical bytes store to one reference and exactly one stored object.
#[tokio::test]
async fn identical_bytes_store_to_equal_reference_and_single_object() {
    let root = tempfile::tempdir().expect("temporary root");
    let store = BlobStore::new(root.path()).expect("owner identity is fixed");

    let first = store
        .store(MEDIA_TYPE, stream_of(b"archive me"))
        .await
        .expect("first store must succeed");
    let second = store
        .store(MEDIA_TYPE, stream_of(b"archive me"))
        .await
        .expect("second store must succeed");

    assert_eq!(
        first, second,
        "identical bytes must produce equal references"
    );
    assert_eq!(first.length_bytes, 10);
    assert_eq!(first.digest.hex.as_str().len(), 64);

    // One object: resolving both references verifies the same bytes.
    store
        .verify(&first)
        .await
        .expect("the first reference must verify");
    store
        .verify(&second)
        .await
        .expect("the second reference must verify");
}

/// The digest names the bytes, proved by hashing independently after a read.
#[tokio::test]
async fn digest_matches_independently_hashed_bytes() {
    use sha2::Digest as _;

    let root = tempfile::tempdir().expect("temporary root");
    let store = BlobStore::new(root.path()).expect("owner identity is fixed");

    let payload = b"deterministic archive payload";
    let reference = store
        .store(MEDIA_TYPE, stream_of(payload))
        .await
        .expect("store must succeed");

    let path = store.verify(&reference).await.expect("verify must succeed");
    let read_back = tokio::fs::read(&path)
        .await
        .expect("stored bytes must be readable through the resolved path");
    let hashed = sha2::Sha256::digest(&read_back);
    assert_eq!(
        hex::encode(hashed),
        reference.digest.hex.as_str(),
        "the reference digest must match independently hashed bytes"
    );
}

/// A failed stream leaves nothing under a final path; retrying succeeds.
#[tokio::test]
async fn interrupted_stream_leaves_no_final_object() {
    let root = tempfile::tempdir().expect("temporary root");
    let store = BlobStore::new(root.path()).expect("owner identity is fixed");

    let failing = Box::pin(stream::iter(vec![
        Ok::<Bytes, std::io::Error>(Bytes::from_static(b"partial")),
        Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "cut")),
    ]))
        as std::pin::Pin<
            Box<dyn futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Send>,
        >;

    let outcome = store.store(MEDIA_TYPE, failing).await;
    assert!(outcome.is_err(), "a broken stream must fail the store");

    let reference = store
        .store(MEDIA_TYPE, stream_of(b"partial"))
        .await
        .expect("a later successful store of different bytes must work");
    store
        .verify(&reference)
        .await
        .expect("the later object must verify");
}

/// Corrupted on-disk bytes resolve as missing, not as changed content.
#[tokio::test]
async fn corrupted_object_resolves_as_missing() {
    let root = tempfile::tempdir().expect("temporary root");
    let store = BlobStore::new(root.path()).expect("owner identity is fixed");

    let reference = store
        .store(MEDIA_TYPE, stream_of(b"intact payload"))
        .await
        .expect("store must succeed");

    let path = store.resolve(&reference).expect("resolution gives a path");
    tokio::fs::write(&path, b"corrupted payload!!")
        .await
        .expect("corrupting the stored bytes must be possible in the test");

    let verified = store.verify(&reference).await;
    assert!(
        verified.is_err(),
        "verification of corrupted bytes must fail"
    );
}

/// A reference owned by another service never resolves here.
#[tokio::test]
async fn foreign_owner_reference_does_not_resolve() {
    let root = tempfile::tempdir().expect("temporary root");
    let store = BlobStore::new(root.path()).expect("owner identity is fixed");

    let mut reference = store
        .store(MEDIA_TYPE, stream_of(b"owned bytes"))
        .await
        .expect("store must succeed");
    reference.owner_service =
        ratatoskr_identifiers::BlobOwner::parse("ratatoskr-extractor").expect("valid owner form");

    let outcome = store.verify(&reference).await;
    assert!(
        outcome.is_err(),
        "a foreign owner must not read this service's objects"
    );
}

/// A media type that is not `type/subtype` is refused before any write, and
/// the store stays usable afterwards.
#[tokio::test]
async fn invalid_media_type_is_rejected() {
    let root = tempfile::tempdir().expect("temporary root");
    let store = BlobStore::new(root.path()).expect("owner identity is fixed");

    let outcome = store.store("not-a-media-type", stream_of(b"x")).await;
    assert!(
        outcome.is_err(),
        "a malformed media type must refuse the store"
    );

    let good = store
        .store(MEDIA_TYPE, stream_of(b"x"))
        .await
        .expect("a valid store after a refusal must succeed");
    store
        .verify(&good)
        .await
        .expect("an object stored after a refusal must verify");
}

#[tokio::test]
async fn erase_is_exact_and_idempotent() {
    let root = tempfile::tempdir().expect("temporary root");
    let store = BlobStore::new(root.path()).expect("owner identity is fixed");
    let erased = store
        .store(MEDIA_TYPE, stream_of(b"exclusive bytes"))
        .await
        .expect("exclusive object stores");
    let sibling = store
        .store(MEDIA_TYPE, stream_of(b"sibling bytes"))
        .await
        .expect("sibling object stores");

    store.erase(&erased).await.expect("first erase succeeds");
    assert!(
        store.verify(&erased).await.is_err(),
        "the exclusively named object must be absent after erasure"
    );
    store.erase(&erased).await.expect("replayed erase succeeds");
    store
        .verify(&sibling)
        .await
        .expect("erasure must not touch a sibling object");
}

#[tokio::test]
async fn erase_refuses_foreign_and_malformed_references() {
    let root = tempfile::tempdir().expect("temporary root");
    let store = BlobStore::new(root.path()).expect("owner identity is fixed");
    let foreign_target = store
        .store(MEDIA_TYPE, stream_of(b"foreign refusal target"))
        .await
        .expect("foreign target stores");
    let malformed_target = store
        .store(MEDIA_TYPE, stream_of(b"malformed refusal target"))
        .await
        .expect("malformed target stores");

    let mut foreign = foreign_target.clone();
    foreign.owner_service =
        ratatoskr_identifiers::BlobOwner::parse("ratatoskr-extractor").expect("valid owner form");
    let foreign_result = store.erase(&foreign).await;
    assert!(foreign_result.is_err(), "foreign erasure must be refused");
    store
        .verify(&foreign_target)
        .await
        .expect("foreign refusal must leave local bytes intact");

    let mut malformed = malformed_target.clone();
    malformed.length_bytes = malformed.length_bytes.saturating_add(1);
    let malformed_result = store.erase(&malformed).await;
    assert!(
        malformed_result.is_err(),
        "a reference contradicting the exact stored file must be refused"
    );
    store
        .verify(&malformed_target)
        .await
        .expect("malformed refusal must leave local bytes intact");
}
