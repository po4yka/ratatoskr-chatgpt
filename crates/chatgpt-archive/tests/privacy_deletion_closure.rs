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
use ratatoskr_chatgpt_archive::BlobStore;
use ratatoskr_chatgpt_archive::privacy_deletion::{
    HANDLED_REFERENCES, PrivacyDeletionScope, PrivacyDeletionService,
};
use ratatoskr_identifiers::BlobRef;
use std::collections::BTreeSet;
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
