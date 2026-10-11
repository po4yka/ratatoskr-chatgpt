//! Privacy deletion removes every row that depends on what it deletes, and keeps rows
//! that another export still evidences (XR-021 CONTRACTS.md section S07). Each test
//! runs on a disposable database.

// Test bodies fail through `expect`/`panic!`; assertions are the contract.
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test failures report through panics"
)]

mod common;

use common::{DisposableDatabase, blob, insert_account, insert_export};
use ratatoskr_chatgpt_archive::privacy_deletion::{
    EraseGate, FinalizationFault, HANDLED_REFERENCES, PrivacyDeletionError, PrivacyDeletionScope,
    PrivacyDeletionService,
};
use ratatoskr_chatgpt_archive::receipt::AcquisitionMode;
use ratatoskr_chatgpt_archive::receipt::pg::PostgresReceiptRepository;
use ratatoskr_chatgpt_archive::receipt::repository::{
    PublishRequest, PublishedExport, ReceiptRepository as _, RepositoryError,
};
use ratatoskr_chatgpt_archive::{BlobStore, InitialImportWorker, ParserRegistry};
use ratatoskr_identifiers::BlobRef;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// The tables a deletion removes rows from. Every foreign key into one of them must
/// either cascade or be named in [`HANDLED_REFERENCES`].
const DELETION_ROOTS: [&str; 6] = [
    "exports",
    "import_runs",
    "accounts",
    "projects",
    "conversations",
    "messages",
];

#[tokio::test]
async fn every_foreign_key_into_deletion_roots_is_handled() {
    let database = DisposableDatabase::create().await;
    let roots: Vec<String> = DELETION_ROOTS
        .iter()
        .map(|root| (*root).to_owned())
        .collect();

    let found: Vec<(String, String)> = sqlx::query_as(
        "SELECT referencing.relname::text, a.attname::text
         FROM pg_constraint c
         JOIN pg_class referencing ON referencing.oid = c.conrelid
         JOIN pg_class referenced ON referenced.oid = c.confrelid
         JOIN pg_namespace n ON n.oid = referenced.relnamespace
         JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = ANY (c.conkey)
         WHERE c.contype = 'f' AND n.nspname = 'chatgpt_archive'
           AND referenced.relname = ANY ($1) AND c.confdeltype <> 'c'
         ORDER BY 1, 2",
    )
    .bind(roots)
    .fetch_all(database.pool())
    .await
    .expect("the constraint catalog is readable");

    let handled: BTreeSet<(&str, &str)> = HANDLED_REFERENCES.iter().copied().collect();
    let unhandled: Vec<String> = found
        .iter()
        .filter(|(table, column)| !handled.contains(&(table.as_str(), column.as_str())))
        .map(|(table, column)| format!("{table}.{column}"))
        .collect();
    assert!(
        unhandled.is_empty(),
        "foreign keys into deletion roots that privacy deletion does not handle: {unhandled:?}"
    );
    let stale: Vec<String> = handled
        .iter()
        .filter(|(table, column)| {
            !found
                .iter()
                .any(|(found_table, found_column)| found_table == table && found_column == column)
        })
        .map(|(table, column)| format!("{table}.{column}"))
        .collect();
    assert!(
        stale.is_empty(),
        "handled references that no longer exist in the schema: {stale:?}"
    );
    database.discard().await;
}

struct Seeded {
    tenant: Uuid,
    archive: Uuid,
    export: Uuid,
    raw: BlobRef,
}

/// An account with one export, its import run, and the two dependents the foreign
/// keys used to strand: a Platform operation binding and a reparse result.
async fn seed_export_with_dependents(
    database: &DisposableDatabase,
    blobs: &BlobStore,
    tenant: Uuid,
    bytes: &'static [u8],
) -> Seeded {
    let raw = blob(blobs, bytes).await;
    let archive = Uuid::now_v7();
    let export = insert_export(database, tenant, archive, &raw).await;
    let run = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chatgpt_archive.import_runs (id, export_id, state)
         VALUES ($1, $2, 'completed')",
    )
    .bind(run)
    .bind(export)
    .execute(database.pool())
    .await
    .expect("the import run is inserted");
    sqlx::query(
        "INSERT INTO chatgpt_archive.platform_operation_imports
         (operation_id, import_run_id, export_id) VALUES ($1, $2, $3)",
    )
    .bind(Uuid::now_v7())
    .bind(run)
    .bind(export)
    .execute(database.pool())
    .await
    .expect("the Platform operation binding is inserted");
    sqlx::query(
        "INSERT INTO chatgpt_archive.reparse_runs
         (id, tenant_id, export_id, parser_name, parser_version, raw_sha256_hex,
          registry_fingerprint, input_projection_fingerprint, status, report)
         VALUES ($1, $2, $3, 'p', '1', $4, $4, $4, 'applied', '{}')",
    )
    .bind(Uuid::now_v7())
    .bind(tenant)
    .bind(export)
    .bind(raw.digest.hex.as_str())
    .execute(database.pool())
    .await
    .expect("the reparse result is inserted");
    Seeded {
        tenant,
        archive,
        export,
        raw,
    }
}

async fn dependents_of(database: &DisposableDatabase, export: Uuid) -> (i64, i64, i64) {
    sqlx::query_as(
        "SELECT
           (SELECT count(*) FROM chatgpt_archive.exports WHERE id = $1),
           (SELECT count(*) FROM chatgpt_archive.platform_operation_imports
            WHERE export_id = $1),
           (SELECT count(*) FROM chatgpt_archive.reparse_runs WHERE export_id = $1)",
    )
    .bind(export)
    .fetch_one(database.pool())
    .await
    .expect("the dependents are countable")
}

#[tokio::test]
async fn archive_scope_completes_with_platform_operation_import_and_reparse_run() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let tenant = insert_account(&database, "archive-scope").await;
    let seeded = seed_export_with_dependents(&database, &blobs, tenant, b"archive scope raw").await;
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

    let report = service
        .execute(request)
        .await
        .expect("the deletion completes instead of failing on a foreign key");

    assert_eq!(dependents_of(&database, seeded.export).await, (0, 0, 0));
    assert_eq!(report.totals.get("platform_operation_import"), Some(&1));
    assert_eq!(report.totals.get("reparse_run"), Some(&1));
    assert!(
        blobs.verify(&seeded.raw).await.is_err(),
        "the exclusive raw archive blob is erased"
    );
    database.discard().await;
}

#[tokio::test]
async fn tenant_scope_completes_with_reparse_run() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let tenant = insert_account(&database, "tenant-scope").await;
    let seeded = seed_export_with_dependents(&database, &blobs, tenant, b"tenant scope raw").await;
    let service = PrivacyDeletionService::new(database.pool().clone(), blobs.clone());
    let request = Uuid::now_v7();
    service
        .plan(seeded.tenant, request, PrivacyDeletionScope::Tenant)
        .await
        .expect("planning succeeds")
        .expect("the owned tenant plans");

    let report = service
        .execute(request)
        .await
        .expect("the tenant deletion completes");

    assert_eq!(dependents_of(&database, seeded.export).await, (0, 0, 0));
    assert_eq!(report.totals.get("reparse_run"), Some(&1));
    let accounts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM chatgpt_archive.accounts WHERE id = $1")
            .bind(seeded.tenant)
            .fetch_one(database.pool())
            .await
            .expect("accounts are countable");
    assert_eq!(accounts, 0, "the account itself is removed");
    database.discard().await;
}

#[tokio::test]
async fn conversation_scope_completes_with_platform_operation_import() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let tenant = insert_account(&database, "conversation-scope").await;
    let seeded =
        seed_export_with_dependents(&database, &blobs, tenant, b"conversation scope raw").await;
    let conversation = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chatgpt_archive.conversations (id, account_id, external_id)
         VALUES ($1, $2, $3)",
    )
    .bind(conversation)
    .bind(tenant)
    .bind(format!("conversation-{conversation}"))
    .execute(database.pool())
    .await
    .expect("the conversation is inserted");
    sqlx::query(
        "INSERT INTO chatgpt_archive.export_entity_observations (export_id, entity_kind, entity_id)
         VALUES ($1, 'conversation', $2)",
    )
    .bind(seeded.export)
    .bind(conversation)
    .execute(database.pool())
    .await
    .expect("the observation is inserted");
    let service = PrivacyDeletionService::new(database.pool().clone(), blobs.clone());
    let request = Uuid::now_v7();
    service
        .plan(
            tenant,
            request,
            PrivacyDeletionScope::Conversation {
                conversation_id: conversation,
            },
        )
        .await
        .expect("planning succeeds")
        .expect("the owned conversation plans");

    let report = service
        .execute(request)
        .await
        .expect("the conversation deletion completes");

    assert_eq!(dependents_of(&database, seeded.export).await, (0, 0, 0));
    assert_eq!(report.totals.get("platform_operation_import"), Some(&1));
    database.discard().await;
}

/// Records that every export observed every entity.
async fn observe(database: &DisposableDatabase, exports: &[Uuid], entities: &[(&str, Uuid)]) {
    for export in exports {
        for (kind, entity) in entities {
            sqlx::query(
                "INSERT INTO chatgpt_archive.export_entity_observations
                 (export_id, entity_kind, entity_id) VALUES ($1, $2, $3)",
            )
            .bind(export)
            .bind(kind)
            .bind(entity)
            .execute(database.pool())
            .await
            .expect("the observation is inserted");
        }
    }
}

#[tokio::test]
async fn retained_project_conversation_and_asset_survive_deletion_of_their_first_seen_export() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let tenant = insert_account(&database, "retained").await;
    let raw_a = blob(&blobs, b"export A raw").await;
    let raw_b = blob(&blobs, b"export B raw").await;
    let archive_a = Uuid::now_v7();
    let export_a = insert_export(&database, tenant, archive_a, &raw_a).await;
    let export_b = insert_export(&database, tenant, Uuid::now_v7(), &raw_b).await;
    assert!(export_a < export_b, "export ids are time ordered");

    let project = Uuid::now_v7();
    let conversation = Uuid::now_v7();
    let asset = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chatgpt_archive.projects
         (id, account_id, external_id, first_seen_export, last_seen_export)
         VALUES ($1, $2, 'project', $3, $4)",
    )
    .bind(project)
    .bind(tenant)
    .bind(export_a)
    .bind(export_b)
    .execute(database.pool())
    .await
    .expect("the project is inserted");
    sqlx::query(
        "INSERT INTO chatgpt_archive.conversations
         (id, project_id, account_id, external_id, first_seen_export, last_seen_export)
         VALUES ($1, $2, $3, 'conversation', $4, $5)",
    )
    .bind(conversation)
    .bind(project)
    .bind(tenant)
    .bind(export_a)
    .bind(export_b)
    .execute(database.pool())
    .await
    .expect("the conversation is inserted");
    sqlx::query(
        "INSERT INTO chatgpt_archive.assets (id, external_id, asset_kind, observed_in)
         VALUES ($1, 'asset', 'uploaded_file', $2)",
    )
    .bind(asset)
    .bind(export_a)
    .execute(database.pool())
    .await
    .expect("the asset is inserted");
    observe(
        &database,
        &[export_a, export_b],
        &[
            ("project", project),
            ("conversation", conversation),
            ("asset", asset),
        ],
    )
    .await;
    let service = PrivacyDeletionService::new(database.pool().clone(), blobs.clone());
    let request = Uuid::now_v7();
    service
        .plan(
            tenant,
            request,
            PrivacyDeletionScope::Archive {
                ai_archive_id: archive_a,
            },
        )
        .await
        .expect("planning succeeds")
        .expect("the owned archive plans");

    service
        .execute(request)
        .await
        .expect("the deletion completes instead of failing on first_seen_export");

    let provenance: (Uuid, Uuid, Uuid, Uuid, Uuid) = sqlx::query_as(
        "SELECT p.first_seen_export, p.last_seen_export,
                c.first_seen_export, c.last_seen_export, a.observed_in
         FROM chatgpt_archive.projects p, chatgpt_archive.conversations c,
              chatgpt_archive.assets a
         WHERE p.id = $1 AND c.id = $2 AND a.id = $3",
    )
    .bind(project)
    .bind(conversation)
    .bind(asset)
    .fetch_one(database.pool())
    .await
    .expect("the retained entities remain");
    assert_eq!(
        provenance,
        (export_b, export_b, export_b, export_b, export_b),
        "provenance is repointed to the retained observing export"
    );
    let deleted_export: i64 =
        sqlx::query_scalar("SELECT count(*) FROM chatgpt_archive.exports WHERE id = $1")
            .bind(export_a)
            .fetch_one(database.pool())
            .await
            .expect("exports are countable");
    assert_eq!(deleted_export, 0);
    database.discard().await;
}

// ---- one blob lock protocol for registration and erasure (XR-021 CONTRACTS.md R2-07) ----

/// Every blob reference any committed row holds must name an object that verifies.
async fn assert_no_dangling_reference(database: &DisposableDatabase, blobs: &BlobStore) {
    for table in ["exports", "extracted_artifacts", "assets"] {
        let references: Vec<serde_json::Value> = sqlx::query_scalar(&format!(
            "SELECT blob_ref FROM chatgpt_archive.{table} WHERE blob_ref IS NOT NULL"
        ))
        .fetch_all(database.pool())
        .await
        .expect("the references are readable");
        for encoded in references {
            let reference: BlobRef =
                serde_json::from_value(encoded).expect("a stored reference decodes");
            assert!(
                blobs.verify(&reference).await.is_ok(),
                "a committed {table} row references a blob that no longer exists"
            );
        }
    }
}

/// A deletion whose rows are already gone and whose bytes are not yet erased.
struct RowsGone {
    service: PrivacyDeletionService,
    request: Uuid,
    seeded: Seeded,
}

async fn rows_gone(database: &DisposableDatabase, blobs: &BlobStore) -> RowsGone {
    let tenant = insert_account(database, "eraser").await;
    let seeded =
        seed_export_with_dependents(database, blobs, tenant, b"bytes two tenants both upload")
            .await;
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
    let interrupted = service
        .execute_with_fault(request, FinalizationFault::AfterRowsCommitted)
        .await;
    assert!(
        matches!(interrupted, Err(PrivacyDeletionError::InjectedFinalization)),
        "the rows commit and the process stops before any byte is erased: {interrupted:?}"
    );
    RowsGone {
        service,
        request,
        seeded,
    }
}

/// Another tenant's receipt of the same bytes, ready to register its reference.
struct SecondUpload {
    repository: PostgresReceiptRepository,
    request: PublishRequest,
    staged: tempfile::NamedTempFile,
}

async fn second_upload(
    database: &DisposableDatabase,
    blobs: &BlobStore,
    raw: &BlobRef,
    bytes: &[u8],
) -> SecondUpload {
    let repository = PostgresReceiptRepository::new(database.pool().clone(), blobs.clone());
    let account = format!("acc-second-{}", Uuid::now_v7());
    let run_id = repository
        .create_run(
            &account,
            &AcquisitionMode::ConsumerExport,
            "application/zip",
        )
        .await
        .expect("a run is created");
    let digest = raw.digest.hex.as_str().to_owned();
    repository
        .record_hash(run_id, digest.clone(), raw.length_bytes)
        .await
        .expect("the digest is recorded");
    let staged = tempfile::NamedTempFile::new().expect("a staging file");
    std::fs::write(staged.path(), bytes).expect("the staging file is written");
    let request = PublishRequest {
        run_id,
        account_external_ref: account,
        mode: AcquisitionMode::ConsumerExport,
        blob_ref_json: serde_json::to_value(raw).expect("a blob reference encodes"),
        sha256_hex: digest,
        byte_length: raw.length_bytes,
        platform_operation: None,
        staged_path: staged.path().to_path_buf(),
    };
    SecondUpload {
        repository,
        request,
        staged,
    }
}

const SHARED_BYTES: &[u8] = b"bytes two tenants both upload";

/// Runs the eraser up to the point where it holds the digest lock after its re-check, starts
/// the second tenant's registration, gives the registration time to run if it can, and then
/// lets the eraser finish. Returns what the registration answered.
async fn race_registration_against_eraser(
    gone: &RowsGone,
    upload: SecondUpload,
    while_held: impl FnOnce(&std::path::Path),
) -> Result<PublishedExport, RepositoryError> {
    let gate = EraseGate::for_digest(gone.seeded.raw.digest.hex.as_str());
    let eraser = gone.service.clone().with_erase_gate(Arc::clone(&gate));
    let request = gone.request;
    let eraser_task = tokio::spawn(async move { eraser.execute(request).await });
    tokio::time::timeout(Duration::from_secs(20), gate.reached())
        .await
        .expect("the eraser reaches the blob");

    while_held(upload.staged.path());
    let registration = tokio::spawn(async move {
        let SecondUpload {
            repository,
            request,
            staged,
        } = upload;
        let result = repository.publish_export(request).await;
        drop(staged);
        result
    });
    tokio::time::sleep(Duration::from_millis(400)).await;

    gate.resume();
    eraser_task
        .await
        .expect("the eraser task ends")
        .expect("the deletion completes");
    registration.await.expect("the registration task ends")
}

/// Interleaving (a): the eraser holds the digest lock after its re-check while another tenant
/// registers a reference to the same digest. The registration waits, finds the blob gone,
/// restores it from its staged bytes, and no committed row references a missing blob.
#[tokio::test]
async fn a_concurrent_import_of_identical_bytes_never_leaves_a_dangling_reference() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let gone = rows_gone(&database, &blobs).await;
    let upload = second_upload(&database, &blobs, &gone.seeded.raw, SHARED_BYTES).await;

    let registered = race_registration_against_eraser(&gone, upload, |_| {}).await;

    registered.expect("the registration restores the blob it needs and succeeds");
    assert_no_dangling_reference(&database, &blobs).await;
    let references: i64 =
        sqlx::query_scalar("SELECT count(*) FROM chatgpt_archive.exports WHERE sha256_hex = $1")
            .bind(gone.seeded.raw.digest.hex.as_str())
            .fetch_one(database.pool())
            .await
            .expect("the exports are countable");
    assert_eq!(references, 1, "the second tenant's export is registered");
    database.discard().await;
}

/// Interleaving (b): the registration commits first, so the eraser's re-check under the lock
/// finds the reference and keeps the blob.
#[tokio::test]
async fn a_registration_that_commits_first_keeps_the_blob_for_the_eraser() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let gone = rows_gone(&database, &blobs).await;
    let upload = second_upload(&database, &blobs, &gone.seeded.raw, SHARED_BYTES).await;
    upload
        .repository
        .publish_export(upload.request)
        .await
        .expect("the registration commits while the blob still exists");

    let report = gone
        .service
        .execute(gone.request)
        .await
        .expect("the deletion completes");

    assert_eq!(
        report.status,
        ratatoskr_chatgpt_archive::privacy_deletion::DeletionStatus::Completed
    );
    assert!(
        blobs.verify(&gone.seeded.raw).await.is_ok(),
        "the eraser keeps a blob another tenant's row references"
    );
    assert_no_dangling_reference(&database, &blobs).await;
    database.discard().await;
}

/// Interleaving (c): the eraser wins and the registration cannot restore the bytes. It fails
/// with a retryable class and registers nothing.
#[tokio::test]
async fn a_registration_that_cannot_restore_the_blob_fails_and_registers_nothing() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let gone = rows_gone(&database, &blobs).await;
    let upload = second_upload(&database, &blobs, &gone.seeded.raw, SHARED_BYTES).await;

    let registered = race_registration_against_eraser(&gone, upload, |staged| {
        std::fs::remove_file(staged)
            .expect("the staged bytes are lost while the eraser holds the lock");
    })
    .await;

    assert!(
        matches!(registered, Err(RepositoryError::BlobUnavailable)),
        "a registration that cannot restore its blob fails with the retryable class: {registered:?}"
    );
    let references: i64 = sqlx::query_scalar("SELECT count(*) FROM chatgpt_archive.exports")
        .fetch_one(database.pool())
        .await
        .expect("the exports are countable");
    assert_eq!(references, 0, "nothing was registered");
    assert_no_dangling_reference(&database, &blobs).await;
    database.discard().await;
}

/// The importer registers extracted artifacts under the same protocol: an artifact blob a
/// concurrent eraser removes is stored again from the bytes the import still holds.
#[tokio::test]
async fn an_import_restores_an_artifact_blob_a_concurrent_eraser_removes() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let worker = || {
        InitialImportWorker::new(
            database.pool().clone(),
            blobs.clone(),
            Arc::new(ParserRegistry::runtime().expect("the runtime registry builds")),
            common::archive_limits(),
        )
    };

    // Tenant A imports an archive, then asks for it to be deleted; its rows are gone.
    let first = common::receive_export(
        database.pool(),
        &blobs,
        &format!("acc-a-{}", Uuid::now_v7()),
        common::conversations_only_zip(),
    )
    .await;
    common::drain(&worker()).await;
    let artifact: serde_json::Value = sqlx::query_scalar(
        "SELECT blob_ref FROM chatgpt_archive.extracted_artifacts WHERE export_id = $1",
    )
    .bind(first.export)
    .fetch_one(database.pool())
    .await
    .expect("the import extracted the one conversations entry");
    let artifact: BlobRef = serde_json::from_value(artifact).expect("the reference decodes");
    let service = PrivacyDeletionService::new(database.pool().clone(), blobs.clone());
    let request = Uuid::now_v7();
    service
        .plan(
            first.account,
            request,
            PrivacyDeletionScope::Archive {
                ai_archive_id: first.archive,
            },
        )
        .await
        .expect("planning succeeds")
        .expect("the owned archive plans");
    let interrupted = service
        .execute_with_fault(request, FinalizationFault::AfterRowsCommitted)
        .await;
    assert!(matches!(
        interrupted,
        Err(PrivacyDeletionError::InjectedFinalization)
    ));

    // Tenant B uploads a different archive that contains the very same entry bytes.
    let second = common::zip_of(&[
        (
            "conversations.json",
            include_bytes!("fixtures/synthetic_conversations.json"),
        ),
        (
            "notes.txt",
            b"an extra entry that makes this archive different",
        ),
    ]);
    common::receive_export(
        database.pool(),
        &blobs,
        &format!("acc-b-{}", Uuid::now_v7()),
        second,
    )
    .await;

    let gate = EraseGate::for_digest(artifact.digest.hex.as_str());
    let eraser = service.clone().with_erase_gate(Arc::clone(&gate));
    let eraser_task = tokio::spawn(async move { eraser.execute(request).await });
    tokio::time::timeout(Duration::from_secs(20), gate.reached())
        .await
        .expect("the eraser reaches the shared artifact");
    let importer = worker();
    let import_task = tokio::spawn(async move { common::drain(&importer).await });
    tokio::time::sleep(Duration::from_millis(600)).await;
    gate.resume();
    eraser_task
        .await
        .expect("the eraser task ends")
        .expect("the deletion completes");
    import_task.await.expect("the import settles");

    let registered: i64 =
        sqlx::query_scalar("SELECT count(*) FROM chatgpt_archive.extracted_artifacts")
            .fetch_one(database.pool())
            .await
            .expect("the artifacts are countable");
    assert!(registered > 0, "the second import registered its artifacts");
    assert_no_dangling_reference(&database, &blobs).await;
    database.discard().await;
}
