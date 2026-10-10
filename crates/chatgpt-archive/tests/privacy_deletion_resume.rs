//! Privacy deletion removes rows before bytes and converges after a crash between the
//! two (XR-021 CONTRACTS.md section S07: rows first, bytes second, complete last).

// Test bodies fail through `expect`/`panic!`; assertions are the contract.
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test failures report through panics"
)]

mod common;

use common::{DisposableDatabase, blob, insert_account, insert_export};
use ratatoskr_chatgpt_archive::BlobStore;
use ratatoskr_chatgpt_archive::privacy_deletion::{
    FinalizationFault, PrivacyDeletionError, PrivacyDeletionScope, PrivacyDeletionService,
};
use ratatoskr_identifiers::BlobRef;
use uuid::Uuid;

/// Every blob a row that still exists references.
async fn referenced_blobs(database: &DisposableDatabase) -> Vec<BlobRef> {
    let encoded: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT blob_ref FROM chatgpt_archive.exports
         UNION ALL SELECT blob_ref FROM chatgpt_archive.extracted_artifacts
         UNION ALL SELECT blob_ref FROM chatgpt_archive.assets WHERE blob_ref IS NOT NULL
         UNION ALL SELECT blob_ref FROM chatgpt_archive.content_parts WHERE blob_ref IS NOT NULL",
    )
    .fetch_all(database.pool())
    .await
    .expect("referencing rows are readable");
    encoded
        .into_iter()
        .map(|value| serde_json::from_value(value).expect("a stored blob reference decodes"))
        .collect()
}

/// An archive with an extracted artifact and one observed conversation.
struct Seeded {
    tenant: Uuid,
    archive: Uuid,
    export: Uuid,
    conversation: Uuid,
    raw: BlobRef,
    extracted: BlobRef,
}

async fn seed(database: &DisposableDatabase, blobs: &BlobStore) -> Seeded {
    let tenant = insert_account(database, "resume").await;
    let raw = blob(blobs, b"resume raw archive").await;
    let extracted = blob(blobs, b"resume extracted evidence").await;
    let archive = Uuid::now_v7();
    let export = insert_export(database, tenant, archive, &raw).await;
    sqlx::query(
        "INSERT INTO chatgpt_archive.extracted_artifacts
         (id, export_id, artifact_ordinal, artifact_kind, blob_ref, sha256_hex, byte_length)
         VALUES ($1, $2, 0, 'entry', $3, $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(export)
    .bind(serde_json::to_value(&extracted).expect("a blob reference encodes"))
    .bind(extracted.digest.hex.as_str())
    .bind(i64::try_from(extracted.length_bytes).expect("a small length"))
    .execute(database.pool())
    .await
    .expect("the extracted artifact is inserted");
    let conversation = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chatgpt_archive.conversations (id, account_id, external_id)
         VALUES ($1, $2, 'resume-conversation')",
    )
    .bind(conversation)
    .bind(tenant)
    .execute(database.pool())
    .await
    .expect("the conversation is inserted");
    sqlx::query(
        "INSERT INTO chatgpt_archive.export_entity_observations (export_id, entity_kind, entity_id)
         VALUES ($1, 'conversation', $2)",
    )
    .bind(export)
    .bind(conversation)
    .execute(database.pool())
    .await
    .expect("the observation is inserted");
    Seeded {
        tenant,
        archive,
        export,
        conversation,
        raw,
        extracted,
    }
}

/// The request status and how many items it still carries.
async fn request_state(database: &DisposableDatabase, request: Uuid) -> (String, i64) {
    sqlx::query_as(
        "SELECT r.status,
                (SELECT count(*) FROM chatgpt_archive.privacy_deletion_items WHERE request_id = $1)
         FROM chatgpt_archive.privacy_deletion_requests r WHERE r.id = $1",
    )
    .bind(request)
    .fetch_one(database.pool())
    .await
    .expect("the request is readable")
}

#[tokio::test]
async fn rows_are_removed_before_bytes_and_a_crash_between_resumes() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let seeded = seed(&database, &blobs).await;
    let service = PrivacyDeletionService::new(database.pool().clone(), blobs.clone());
    let request = Uuid::now_v7();
    service
        .plan(
            seeded.tenant,
            request,
            PrivacyDeletionScope::Archive {
                ai_archive_id: seeded.archive,
            },
        )
        .await
        .expect("planning succeeds")
        .expect("the owned archive plans");

    let crashed = service
        .execute_with_fault(request, FinalizationFault::AfterRowsCommitted)
        .await;

    assert!(
        matches!(crashed, Err(PrivacyDeletionError::InjectedFinalization)),
        "the fault fires after the rows commit: {crashed:?}"
    );
    let remaining: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM chatgpt_archive.exports WHERE id = $1),
                (SELECT count(*) FROM chatgpt_archive.conversations WHERE id = $2)",
    )
    .bind(seeded.export)
    .bind(seeded.conversation)
    .fetch_one(database.pool())
    .await
    .expect("rows are countable");
    assert_eq!(remaining, (0, 0), "the rows are gone");
    let (status, items) = request_state(&database, request).await;
    assert_eq!(status, "purging");
    assert!(
        items > 0,
        "the items carry the blob references phase B erases"
    );
    blobs
        .verify(&seeded.raw)
        .await
        .expect("the raw blob is still on disk until the bytes phase");
    for reference in referenced_blobs(&database).await {
        blobs
            .verify(&reference)
            .await
            .expect("no remaining row references a missing blob");
    }

    let report = service
        .execute(request)
        .await
        .expect("the resume completes");

    assert!(
        blobs.verify(&seeded.raw).await.is_err(),
        "the raw blob is erased"
    );
    assert!(
        blobs.verify(&seeded.extracted).await.is_err(),
        "the extracted blob is erased"
    );
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chatgpt_archive.privacy_deletion_audits WHERE request_id = $1",
    )
    .bind(request)
    .fetch_one(database.pool())
    .await
    .expect("audits are countable");
    assert_eq!(audits, 1);
    assert_eq!(
        request_state(&database, request).await,
        ("completed".to_owned(), 0)
    );

    let replay = service.execute(request).await.expect("a replay succeeds");
    assert_eq!(
        replay, report,
        "a third execution returns the stored report"
    );
    database.discard().await;
}
