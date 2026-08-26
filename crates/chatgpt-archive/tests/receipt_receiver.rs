//! Contract tests for the streaming archive receiver.
//!
//! Every test drives [`ArchiveReceiver`] through the public API against the
//! hand-written fake repository and real temp directories; `PostgreSQL`
//! correctness lives in `receipt_repository_pg.rs`.

// Test bodies fail through `panic!`/`expect`; assertions are the contract,
// and the stream/fixture helpers sit outside `#[test]` functions.
#![allow(clippy::expect_used, reason = "test failures report through panics")]
#![allow(clippy::panic, reason = "test failures report through panics")]

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_core::Stream;
use ratatoskr_chatgpt_archive::BlobStore;
use ratatoskr_chatgpt_archive::receipt::auth::TenantPrincipal;
use ratatoskr_chatgpt_archive::receipt::state::ImportState;
use ratatoskr_chatgpt_archive::receipt::{
    AcquisitionMode, ArchiveReceiver, ReceiptError, ReceiptOutcome,
};
use ratatoskr_chatgpt_archive::test_support::{FakeReceiptRepository, FakeRun};
use sha2::Digest as _;
use uuid::Uuid;

/// The stream error tests inject.
#[derive(Debug)]
pub struct TestStreamError;

impl core::fmt::Display for TestStreamError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("the test stream broke")
    }
}

impl std::error::Error for TestStreamError {}

/// A bounded in-memory byte stream that counts how many chunks were pulled.
struct CountingStream {
    chunks: Vec<Bytes>,
    next: usize,
    pulled: Arc<AtomicUsize>,
    fail_at: Option<usize>,
}

impl CountingStream {
    fn new(payloads: &[&[u8]]) -> (Self, Arc<AtomicUsize>) {
        let pulled = Arc::new(AtomicUsize::new(0));
        (
            Self {
                chunks: payloads
                    .iter()
                    .map(|chunk| Bytes::copy_from_slice(chunk))
                    .collect(),
                next: 0,
                pulled: Arc::clone(&pulled),
                fail_at: None,
            },
            pulled,
        )
    }

    fn failing_after(payloads: &[&[u8]]) -> (Self, Arc<AtomicUsize>) {
        let count = payloads.len();
        let (mut stream, pulled) = Self::new(payloads);
        stream.fail_at = Some(count);
        (stream, pulled)
    }
}

impl Stream for CountingStream {
    type Item = Result<Bytes, TestStreamError>;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(fail_at) = self.fail_at
            && self.next == fail_at
        {
            self.next += 1;
            return Poll::Ready(Some(Err(TestStreamError)));
        }
        let index = self.next;
        let Some(chunk) = self.chunks.get(index).cloned() else {
            return Poll::Ready(None);
        };
        self.next = index + 1;
        self.pulled.fetch_add(1, Ordering::SeqCst);
        Poll::Ready(Some(Ok(chunk)))
    }
}

const MEDIA_TYPE: &str = "application/zip";

fn principal(reference: &str) -> TenantPrincipal {
    TenantPrincipal {
        account_external_ref: reference.to_owned(),
    }
}

struct Fixture {
    receiver: ArchiveReceiver,
    repository: Arc<FakeReceiptRepository>,
    #[allow(dead_code)]
    blob_root: tempfile::TempDir,
    #[allow(dead_code)]
    staging: tempfile::TempDir,
}

fn fixture(max_archive_bytes: u64) -> Fixture {
    let blob_root = tempfile::tempdir().expect("blob root");
    let staging = tempfile::tempdir().expect("staging root");
    let blob = BlobStore::new(blob_root.path()).expect("blob store");
    let repository = Arc::new(FakeReceiptRepository::new());
    let receiver = ArchiveReceiver::new(
        blob,
        Arc::clone(&repository) as Arc<dyn ratatoskr_chatgpt_archive::ReceiptRepository>,
        staging.path().to_path_buf(),
        max_archive_bytes,
    )
    .expect("receiver");
    Fixture {
        receiver,
        repository,
        blob_root,
        staging,
    }
}

async fn receive_fixture_bytes(
    fixture: &Fixture,
    tenant: &str,
    payloads: &[&[u8]],
) -> Result<ReceiptOutcome, ReceiptError> {
    let (stream, _) = CountingStream::new(payloads);
    let declared: u64 = payloads.iter().map(|chunk| chunk.len() as u64).sum();
    fixture
        .receiver
        .receive(
            &principal(tenant),
            AcquisitionMode::ConsumerExport,
            MEDIA_TYPE,
            Some(declared),
            stream,
        )
        .await
}

/// A multi-chunk upload hashes incrementally, stages, publishes, and records
/// evidence whose digest equals an independent hash of the bytes.
#[tokio::test]
async fn chunked_upload_hashes_incrementally_and_records_verified_evidence() {
    let fixture = fixture(u64::MAX);
    let payload: Vec<u8> = (0..64 * 1024_u32 * 3)
        .map(|index| (index % 251) as u8)
        .collect();
    let (left, tail) = payload.split_at(64 * 1024 + 7);
    let (middle, right) = tail.split_at(64 * 1024);

    let outcome = receive_fixture_bytes(&fixture, "acc-one", &[left, middle, right]).await;

    let ReceiptOutcome::Stored {
        export_id,
        sha256_hex,
        byte_length,
    } = outcome.expect("a healthy upload must be stored")
    else {
        panic!("first receipt of fresh content must be stored");
    };
    let expected = hex::encode(sha2::Sha256::digest(&payload));
    assert_eq!(sha256_hex, expected);
    assert_eq!(byte_length, payload.len() as u64);

    let exports = fixture.repository.exports_snapshot();
    assert_eq!(exports.len(), 1);
    assert_eq!(exports[0].export_id, export_id);
    assert_eq!(exports[0].account_external_ref, "acc-one");
    assert_eq!(exports[0].sha256_hex, expected);
    // The blob reference names content addressing, not client data.
    assert!(
        exports[0].blob_ref_json.to_string().contains(&expected),
        "the blob reference must carry the digest"
    );

    let run = fixture
        .repository
        .runs_snapshot()
        .into_iter()
        .next()
        .expect("one run")
        .1;
    assert_eq!(run.state, ImportState::Stored);
    assert_eq!(run.sha256_hex.as_deref(), Some(expected.as_str()));
}

/// A declared length over the cap is refused before any body byte is read,
/// and the durable anchor records the failure.
#[tokio::test]
async fn declared_length_over_cap_is_refused_before_reading() {
    let fixture = fixture(1024);
    let (stream, pulled) = CountingStream::new(&[b"x"]);
    let error = fixture
        .receiver
        .receive(
            &principal("acc-one"),
            AcquisitionMode::ConsumerExport,
            MEDIA_TYPE,
            Some(2048),
            stream,
        )
        .await
        .expect_err("an oversized declaration must be refused");
    assert!(matches!(error, ReceiptError::DeclaredSizeExceeded));
    assert_eq!(pulled.load(Ordering::SeqCst), 0, "no body byte may be read");
    let run = fixture
        .repository
        .runs_snapshot()
        .into_iter()
        .next()
        .expect("run")
        .1;
    assert_eq!(run.state, ImportState::Failed);
    assert!(fixture.repository.exports_snapshot().is_empty());
}

/// A size bomb that never declares its size stops at the cap within one
/// chunk of overshoot, publishes nothing, and fails durably.
#[tokio::test]
async fn streamed_overrun_aborts_at_cap_and_fails_run_durably() {
    let chunk = [0u8; 512];
    let fixture = fixture(1024);
    let (stream, pulled) = CountingStream::new(&[&chunk, &chunk, &chunk, &chunk]);
    let error = fixture
        .receiver
        .receive(
            &principal("acc-one"),
            AcquisitionMode::ConsumerExport,
            MEDIA_TYPE,
            None,
            stream,
        )
        .await
        .expect_err("an undeclared overrun must be refused");
    assert!(matches!(error, ReceiptError::StreamOvergrown));
    // At most one chunk past the cap is ever pulled.
    assert!(
        pulled.load(Ordering::SeqCst) <= 3,
        "consumption must stop near the cap, pulled {}",
        pulled.load(Ordering::SeqCst)
    );
    assert!(fixture.repository.exports_snapshot().is_empty());
    assert!(fixture.repository.publishes().is_empty());
    let run = fixture
        .repository
        .runs_snapshot()
        .into_iter()
        .next()
        .expect("run")
        .1;
    assert_eq!(run.state, ImportState::Failed);
}

/// A stream that dies mid-body leaves no published object and no export row,
/// but leaves a durably failed run.
#[tokio::test]
async fn truncated_stream_publishes_nothing_but_fails_run_durably() {
    let fixture = fixture(u64::MAX);
    let (stream, _) = CountingStream::failing_after(&[b"half-a-body"]);
    let error = fixture
        .receiver
        .receive(
            &principal("acc-one"),
            AcquisitionMode::ConsumerExport,
            MEDIA_TYPE,
            None,
            stream,
        )
        .await
        .expect_err("a truncated stream cannot complete");
    assert!(matches!(error, ReceiptError::StreamFailed(_)));
    assert!(fixture.repository.exports_snapshot().is_empty());
    assert!(fixture.repository.publishes().is_empty());
    let run = fixture
        .repository
        .runs_snapshot()
        .into_iter()
        .next()
        .expect("run")
        .1;
    assert_eq!(run.state, ImportState::Failed);
}

/// Re-receiving identical bytes answers duplicate, keeps one export row, and
/// never touches blob storage again.
#[tokio::test]
async fn identical_reupload_answers_duplicate_without_new_rows() {
    let fixture = fixture(u64::MAX);
    let first = receive_fixture_bytes(&fixture, "acc-one", &[b"a-body", b"-more"]).await;
    let second = receive_fixture_bytes(&fixture, "acc-one", &[b"a-body", b"-more"]).await;

    let ReceiptOutcome::Stored { export_id, .. } = first.expect("fresh content stores") else {
        panic!("expected stored");
    };
    let ReceiptOutcome::Duplicate {
        existing_export_id, ..
    } = second.expect("identical content answers duplicate")
    else {
        panic!("expected duplicate");
    };
    assert_eq!(existing_export_id, export_id);
    assert_eq!(fixture.repository.exports_snapshot().len(), 1);
    assert_eq!(
        fixture.repository.publishes().len(),
        1,
        "duplicate detection precedes publishing"
    );
}

/// Different content under the same tenant stores as a distinct export.
#[tokio::test]
async fn different_content_stores_as_new_export() {
    let fixture = fixture(u64::MAX);
    let first = receive_fixture_bytes(&fixture, "acc-one", &[b"content-one"]).await;
    let second = receive_fixture_bytes(&fixture, "acc-one", &[b"content-two"]).await;

    let ReceiptOutcome::Stored {
        export_id: first_id,
        ..
    } = first.expect("first stores")
    else {
        panic!("expected stored");
    };
    let ReceiptOutcome::Stored {
        export_id: second_id,
        ..
    } = second.expect("second stores")
    else {
        panic!("expected stored");
    };
    assert_ne!(first_id, second_id);
    assert_eq!(fixture.repository.exports_snapshot().len(), 2);
}

/// Equal digests under different tenants each get their own export row.
#[tokio::test]
async fn equal_digests_across_tenants_store_separately() {
    let fixture = fixture(u64::MAX);
    let mine = receive_fixture_bytes(&fixture, "acc-one", &[b"shared-bytes"]).await;
    let theirs = receive_fixture_bytes(&fixture, "acc-two", &[b"shared-bytes"]).await;

    let ReceiptOutcome::Stored {
        export_id: mine_id, ..
    } = mine.expect("own tenant stores")
    else {
        panic!("expected stored");
    };
    let ReceiptOutcome::Stored {
        export_id: theirs_id,
        ..
    } = theirs.expect("other tenant stores separately")
    else {
        panic!("expected stored");
    };
    assert_ne!(mine_id, theirs_id);
    let exports = fixture.repository.exports_snapshot();
    assert_eq!(exports.len(), 2);
    assert_eq!(exports[0].sha256_hex, exports[1].sha256_hex);
}

// --- resume after interruption ---

fn seed_run(fixture: &Fixture, run: FakeRun) -> Uuid {
    fixture.repository.seed_run(run)
}

fn staging_path(fixture: &Fixture, run_id: Uuid) -> std::path::PathBuf {
    fixture.staging.path().join(format!("{run_id}.part"))
}

/// A crashed run at `hashed` with intact staging resumes to `stored` without
/// new bytes.
#[tokio::test]
async fn hashed_run_resumes_to_stored_without_new_bytes() {
    let fixture = fixture(u64::MAX);
    let body = b"interrupted-archive-bytes";
    let digest = hex::encode(sha2::Sha256::digest(body));
    let run_id = seed_run(
        &fixture,
        FakeRun::hashed("acc-one", &digest, body.len() as u64),
    );
    std::fs::write(staging_path(&fixture, run_id), body).expect("staging write");

    let outcome = fixture
        .receiver
        .resume(run_id)
        .await
        .expect("resume succeeds");
    let Some(ReceiptOutcome::Stored {
        export_id,
        sha256_hex,
        byte_length,
    }) = outcome
    else {
        panic!("hashed resume must reach stored");
    };
    assert_eq!(sha256_hex, digest);
    assert_eq!(byte_length, body.len() as u64);
    assert_eq!(fixture.repository.publishes().len(), 1);
    assert_eq!(
        fixture.repository.run(run_id).expect("run").state,
        ImportState::Stored
    );
    assert_eq!(
        fixture.repository.exports_snapshot()[0].export_id,
        export_id
    );
    assert!(
        !staging_path(&fixture, run_id).exists(),
        "staging is consumed"
    );
}

/// A crashed run at `received` re-verifies its staging file to advance.
#[tokio::test]
async fn received_run_reverifies_staging_then_advances() {
    let fixture = fixture(u64::MAX);
    let body = b"bytes-that-were-staged";
    let run_id = seed_run(&fixture, FakeRun::received("acc-one"));
    std::fs::write(staging_path(&fixture, run_id), body).expect("staging write");

    let outcome = fixture
        .receiver
        .resume(run_id)
        .await
        .expect("resume succeeds");
    let Some(ReceiptOutcome::Stored {
        sha256_hex,
        byte_length,
        ..
    }) = outcome
    else {
        panic!("received resume must reach stored");
    };
    assert_eq!(sha256_hex, hex::encode(sha2::Sha256::digest(body)));
    assert_eq!(byte_length, body.len() as u64);
}

/// A non-terminal run whose staging evidence disappeared fails durably.
#[tokio::test]
async fn missing_staging_fails_the_run_durably() {
    let fixture = fixture(u64::MAX);
    let run_id = seed_run(&fixture, FakeRun::hashed("acc-one", &"a".repeat(64), 10));

    let error = fixture
        .receiver
        .resume(run_id)
        .await
        .expect_err("lost staging cannot resume");
    assert!(matches!(error, ReceiptError::StagingEvidenceLost));
    assert_eq!(
        fixture.repository.run(run_id).expect("run").state,
        ImportState::Failed
    );
    assert!(fixture.repository.publishes().is_empty());
}

/// Resuming a run already at a terminal state changes nothing.
#[tokio::test]
async fn resuming_a_terminal_run_changes_nothing() {
    let fixture = fixture(u64::MAX);
    let mut failed = FakeRun::received("acc-one");
    failed.state = ImportState::Failed;
    let run_id = seed_run(&fixture, failed);

    let outcome = fixture
        .receiver
        .resume(run_id)
        .await
        .expect("terminal is a no-op");
    assert!(outcome.is_none(), "no new outcome is manufactured");
    assert_eq!(
        fixture.repository.run(run_id).expect("run").state,
        ImportState::Failed
    );
}
