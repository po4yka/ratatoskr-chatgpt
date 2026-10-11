//! The import state machine: declared states, legal transitions, terminal
//! protection. Pure logic; persistence guards live with the repository.

#![allow(clippy::expect_used, reason = "test failures report through panics")]

use ratatoskr_chatgpt_archive::receipt::state::{ImportState, TransitionError};

#[test]
fn happy_path_advances_stage_by_stage_and_finishes_on_terminal() {
    let mut run = ImportState::Received;
    for stage in [
        ImportState::Hashed,
        ImportState::Stored,
        ImportState::Inspected,
        ImportState::Parsed,
        ImportState::Reconciled,
    ] {
        run = run.advance(stage).expect("forward progress must be legal");
    }
    assert_eq!(run, ImportState::Reconciled);
    run = run
        .advance(ImportState::Completed)
        .expect("reconciled completes");
    assert_eq!(run, ImportState::Completed);
}

#[test]
fn reconciled_may_also_settle_as_partial() {
    let mut run = ImportState::Received;
    for stage in [
        ImportState::Hashed,
        ImportState::Stored,
        ImportState::Inspected,
        ImportState::Parsed,
        ImportState::Reconciled,
        ImportState::Partial,
    ] {
        run = run.advance(stage).expect("the chain to partial is legal");
    }
    assert_eq!(run, ImportState::Partial);
}

#[test]
fn skipping_a_stage_is_refused() {
    let error = ImportState::Received
        .advance(ImportState::Stored)
        .expect_err("received cannot jump straight to stored");
    assert!(matches!(error, TransitionError::IllegalJump { .. }));
    // And so cannot parsed, which sits two further along the chain.
    let error = ImportState::Hashed
        .advance(ImportState::Parsed)
        .expect_err("hashed cannot jump to parsed");
    assert!(matches!(error, TransitionError::IllegalJump { .. }));
}

#[test]
fn failed_is_reachable_from_every_non_terminal_state() {
    for stage in [
        ImportState::Received,
        ImportState::Hashed,
        ImportState::Stored,
        ImportState::Inspected,
        ImportState::Parsed,
        ImportState::Reconciled,
    ] {
        let failed = stage
            .clone()
            .advance(ImportState::Failed)
            .expect("any non-terminal stage may fail");
        assert_eq!(failed, ImportState::Failed, "from {stage:?}");
    }
}

#[test]
fn terminal_states_accept_no_transition() {
    for terminal in [
        ImportState::Completed,
        ImportState::Partial,
        ImportState::Failed,
        ImportState::Duplicate,
        ImportState::Quarantined,
    ] {
        let error = terminal
            .clone()
            .advance(ImportState::Received)
            .expect_err("a terminal run never advances");
        assert!(
            matches!(error, TransitionError::AlreadyTerminal { .. }),
            "from {terminal:?}"
        );
        // Terminality holds even toward sibling terminals such as failed.
        let error = terminal
            .clone()
            .advance(ImportState::Failed)
            .expect_err("a terminal run cannot be re-classified");
        assert!(matches!(error, TransitionError::AlreadyTerminal { .. }));
    }
}

#[test]
fn duplicate_is_entered_only_from_hashed() {
    let duplicated = ImportState::Hashed
        .advance(ImportState::Duplicate)
        .expect("the duplicate check fires exactly at hashed");
    assert_eq!(duplicated, ImportState::Duplicate);

    let error = ImportState::Stored
        .advance(ImportState::Duplicate)
        .expect_err("after storage the content is no longer a duplicate question");
    assert!(matches!(error, TransitionError::IllegalJump { .. }));
}

#[test]
fn states_spell_exactly_the_declared_set() {
    let declared = [
        ("received", ImportState::Received),
        ("hashed", ImportState::Hashed),
        ("stored", ImportState::Stored),
        ("inspected", ImportState::Inspected),
        ("parsed", ImportState::Parsed),
        ("reconciled", ImportState::Reconciled),
        ("completed", ImportState::Completed),
        ("partial", ImportState::Partial),
        ("failed", ImportState::Failed),
        ("duplicate", ImportState::Duplicate),
        ("quarantined", ImportState::Quarantined),
    ];
    for (spelling, state) in declared {
        assert_eq!(state.as_str(), spelling);
        assert_eq!(ImportState::parse(spelling), Some(state));
    }
    assert_eq!(ImportState::parse("unknown_stage"), None);
    assert_eq!(
        ImportState::parse("RECEIVED"),
        None,
        "spelling is lowercase"
    );
}

// ---- XR-021 CONTRACTS.md R3-04 rule 2 and R3-07 rule 2: failed imports ----

mod common;

use std::sync::Arc;

use common::{DisposableDatabase, archive_limits, drain, receive_export_for};
use ratatoskr_chatgpt_archive::receipt::PlatformOperation;
use ratatoskr_chatgpt_archive::receipt::pg::PostgresReceiptRepository;
use ratatoskr_chatgpt_archive::receipt::repository::ReceiptRepository as _;
use ratatoskr_chatgpt_archive::{BlobStore, InitialImportWorker, ParserRegistry};
use uuid::Uuid;

/// Bytes the receipt admits (it stores whatever Edge verified) and the importer can never parse.
const CORRUPT: &[u8] = b"not a zip archive";

fn worker(database: &DisposableDatabase, blobs: &BlobStore) -> InitialImportWorker {
    InitialImportWorker::new(
        database.pool().clone(),
        blobs.clone(),
        Arc::new(ParserRegistry::runtime().expect("the runtime registry builds")),
        archive_limits(),
    )
}

/// The queued `platform.operation.reported.v1` payloads for one operation.
async fn reports(database: &DisposableDatabase, operation: Uuid) -> Vec<serde_json::Value> {
    sqlx::query_scalar(
        "SELECT payload FROM chatgpt_archive.outbox_events
         WHERE event_type = 'platform.operation.reported.v1' AND aggregate_id = $1 ORDER BY id",
    )
    .bind(operation)
    .fetch_all(database.pool())
    .await
    .expect("the outbox is readable")
}

#[tokio::test]
async fn a_second_upload_of_bytes_that_failed_permanently_is_reported_failed() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let first = Uuid::now_v7();
    let received = receive_export_for(
        database.pool(),
        &blobs,
        "acc-failed",
        CORRUPT.to_vec(),
        Some(PlatformOperation {
            operation_id: first,
        }),
    )
    .await;
    let worker = worker(&database, &blobs);
    drain(&worker).await;
    let state: String =
        sqlx::query_scalar("SELECT state FROM chatgpt_archive.import_runs WHERE export_id = $1")
            .bind(received.export)
            .fetch_one(database.pool())
            .await
            .expect("the run is readable");
    assert_eq!(state, "failed", "the corrupt archive failed permanently");
    assert_eq!(reports(&database, first).await.len(), 1);

    // The same bytes arrive again under a new operation, as a duplicate upload does.
    let second = Uuid::now_v7();
    PostgresReceiptRepository::new(database.pool().clone(), blobs.clone())
        .bind_platform_operation(
            received.export,
            PlatformOperation {
                operation_id: second,
            },
        )
        .await
        .expect("the duplicate binds to the existing export");
    assert!(reports(&database, second).await.is_empty());

    let processed = worker
        .process_pending_once()
        .await
        .expect("the pass succeeds");

    assert_eq!(
        processed, 1,
        "the failed run owes the new operation a report"
    );
    let queued = reports(&database, second).await;
    assert_eq!(queued.len(), 1, "exactly one failed report is queued");
    assert_eq!(queued[0]["payload"]["status"], "failed");
    assert_eq!(
        queued[0]["payload"]["error"]["code"],
        "chatgpt.archive.invalid"
    );
    assert_eq!(queued[0]["payload"]["error"]["retryable"], false);
    assert_eq!(
        worker
            .process_pending_once()
            .await
            .expect("the pass succeeds"),
        0,
        "a further pass finds nothing to report"
    );
    assert_eq!(reports(&database, second).await.len(), 1);
    database.discard().await;
}

#[tokio::test]
async fn a_permanent_import_failure_is_logged_with_a_class() {
    let logs = common::logs::ThreadLogs::install();
    logs.take_lines();
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    receive_export_for(
        database.pool(),
        &blobs,
        "acc-logged",
        CORRUPT.to_vec(),
        None,
    )
    .await;
    let run: Uuid = sqlx::query_scalar("SELECT id FROM chatgpt_archive.import_runs")
        .fetch_one(database.pool())
        .await
        .expect("the run exists");

    drain(&worker(&database, &blobs)).await;

    let lines: Vec<String> = logs
        .take_lines()
        .into_iter()
        .filter(|line| line.contains("import failed"))
        .collect();
    assert_eq!(lines.len(), 1, "one failure, one line: {lines:?}");
    let line = lines.first().expect("one line");
    assert!(line.contains("WARN"), "{line}");
    assert!(line.contains("class=\"archive_invalid\""), "{line}");
    assert!(
        line.contains(&run.to_string()),
        "the run id names the import: {line}"
    );
    assert!(
        !line.contains("not a zip"),
        "the line carries no archive content: {line}"
    );
    database.discard().await;
}

// ---- XR-021 CONTRACTS.md R3-05: an import can be complete ----

const ARCHIVE_CONVERSATIONS: &[u8] =
    include_bytes!("fixtures/synthetic_archive_conversations.json");
const NOTE: &[u8] = include_bytes!("fixtures/assets/note.txt");

/// The shared project and canvas fixtures carry a `future_*` field on purpose, which is a real
/// gap; these are the same documents without it, so the only gaps a test sees are its own.
fn clean_projects() -> Vec<u8> {
    serde_json::json!([{
        "id": "project-alpha",
        "title": "Synthetic project",
        "description": "Invented fixture only",
        "instructions": [
            { "id": "instruction-alpha", "kind": "instruction", "content": "Preserve evidence." }
        ],
        "conversation_ids": ["conversation-project"],
        "asset_ids": ["asset-uploaded"]
    }])
    .to_string()
    .into_bytes()
}

fn clean_canvas() -> Vec<u8> {
    serde_json::json!([{
        "id": "canvas-alpha",
        "project_id": "project-alpha",
        "conversation_id": "conversation-project",
        "content": ["# Inert Canvas", { "kind": "code", "text": "never execute" }]
    }])
    .to_string()
    .into_bytes()
}

/// The `assets.json` that declares `NOTE` under `assets/note.txt` with its true digest.
fn note_declaration() -> Vec<u8> {
    use sha2::Digest as _;
    serde_json::json!([{
        "id": "asset-uploaded",
        "kind": "uploaded",
        "display_name": "note.txt",
        "archive_path": "assets/note.txt",
        "media_type": "text/plain",
        "length_bytes": NOTE.len(),
        "sha256": hex::encode(sha2::Sha256::digest(NOTE)),
    }])
    .to_string()
    .into_bytes()
}

/// What an import of `bytes` left behind: the completeness report row and the operation report.
struct Imported {
    run_state: String,
    completeness: String,
    gaps: Vec<String>,
    asset_count: i64,
    report: serde_json::Value,
}

async fn import(bytes: Vec<u8>) -> Imported {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let operation = Uuid::now_v7();
    let received = receive_export_for(
        database.pool(),
        &blobs,
        "acc-complete",
        bytes,
        Some(PlatformOperation {
            operation_id: operation,
        }),
    )
    .await;
    drain(&worker(&database, &blobs)).await;
    let run_state: String =
        sqlx::query_scalar("SELECT state FROM chatgpt_archive.import_runs WHERE export_id = $1")
            .bind(received.export)
            .fetch_one(database.pool())
            .await
            .expect("the run is readable");
    let completeness: Option<(String, serde_json::Value, serde_json::Value)> =
        sqlx::query_as("SELECT status, gaps, counts FROM chatgpt_archive.completeness_reports")
            .fetch_optional(database.pool())
            .await
            .expect("the completeness report is readable");
    let (completeness, gaps, counts) = completeness
        .unwrap_or_else(|| (String::new(), serde_json::json!([]), serde_json::json!({})));
    let report = reports(&database, operation)
        .await
        .into_iter()
        .next()
        .expect("the operation owes one report");
    let imported = Imported {
        run_state,
        completeness,
        gaps: gaps
            .as_array()
            .expect("gaps are an array")
            .iter()
            .map(|gap| gap["gap_kind"].as_str().expect("a gap kind").to_owned())
            .collect(),
        asset_count: counts
            .get("assets")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        report,
    };
    database.discard().await;
    imported
}

#[tokio::test]
async fn an_export_with_only_conversations_is_complete() {
    let imported = import(common::single_message_zip(
        "conversation-1",
        "Hello archive",
    ))
    .await;

    assert!(imported.gaps.is_empty(), "no gap: {:?}", imported.gaps);
    assert_eq!(imported.run_state, "completed");
    assert_eq!(imported.completeness, "complete");
    assert_eq!(imported.report["payload"]["status"], "succeeded");
    assert!(
        imported.report["payload"]["warnings"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "no warning on a complete import: {}",
        imported.report["payload"]["warnings"]
    );
}

#[tokio::test]
async fn an_export_with_projects_canvas_and_verified_assets_is_complete() {
    let imported = import(common::zip_of(&[
        ("conversations.json", ARCHIVE_CONVERSATIONS),
        ("projects.json", &clean_projects()),
        ("canvas.json", &clean_canvas()),
        ("assets.json", &note_declaration()),
        ("assets/note.txt", NOTE),
    ]))
    .await;

    assert!(imported.gaps.is_empty(), "no gap: {:?}", imported.gaps);
    assert_eq!(imported.run_state, "completed");
    assert_eq!(imported.completeness, "complete");
    assert_eq!(imported.asset_count, 1, "the verified asset is imported");
    assert_eq!(imported.report["payload"]["status"], "succeeded");
}

#[tokio::test]
async fn a_declared_asset_with_no_file_is_a_missing_file_gap() {
    let imported = import(common::zip_of(&[
        ("conversations.json", ARCHIVE_CONVERSATIONS),
        ("projects.json", &clean_projects()),
        ("assets.json", &note_declaration()),
    ]))
    .await;

    assert_eq!(imported.run_state, "partial");
    assert_eq!(imported.completeness, "structurally_partial");
    assert_eq!(imported.gaps, ["missing_file"]);
    assert_eq!(imported.asset_count, 0);
    assert_eq!(imported.report["payload"]["status"], "partially_succeeded");
    let warnings = imported.report["payload"]["warnings"]
        .as_array()
        .expect("warnings are an array");
    assert_eq!(warnings.len(), 1, "exactly one incomplete-import warning");
    assert_eq!(warnings[0]["code"], "ai_archive.import.incomplete");
}

#[tokio::test]
async fn a_malformed_assets_json_fails_permanently() {
    let imported = import(common::zip_of(&[
        ("conversations.json", ARCHIVE_CONVERSATIONS),
        ("assets.json", b"this is not json"),
    ]))
    .await;

    assert_eq!(imported.run_state, "failed");
    assert_eq!(imported.report["payload"]["status"], "failed");
    assert_eq!(
        imported.report["payload"]["error"]["code"],
        "chatgpt.archive.invalid"
    );
}

#[tokio::test]
async fn an_unrecognized_record_is_still_a_gap() {
    // The shared fixture carries a conversation field this parser does not model.
    let imported = import(common::conversations_only_zip()).await;

    assert_eq!(imported.completeness, "structurally_partial");
    assert_eq!(imported.gaps, ["unrecognized_record"]);
    assert_eq!(imported.report["payload"]["status"], "partially_succeeded");
}

// ---- XR-021 CONTRACTS.md R3-04 rule 2: a storage fault leaves the import retryable ----

/// Makes the store's staging path a regular file, which no write can pass, and puts it back.
struct StagingFault(std::path::PathBuf);

impl StagingFault {
    fn inject(root: &std::path::Path) -> Self {
        let staging = root.join("staging");
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::write(&staging, b"not a directory").expect("the fault can be injected");
        Self(staging)
    }

    fn clear(&self) {
        std::fs::remove_file(&self.0).expect("the fault can be cleared");
        std::fs::create_dir_all(&self.0).expect("staging is a directory again");
    }
}

#[tokio::test]
async fn a_storage_fault_during_an_assets_import_is_retried_not_failed() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let operation = Uuid::now_v7();
    let received = receive_export_for(
        database.pool(),
        &blobs,
        "acc-storage-fault",
        common::zip_of(&[
            ("conversations.json", ARCHIVE_CONVERSATIONS),
            ("assets.json", &note_declaration()),
            ("assets/note.txt", NOTE),
        ]),
        Some(PlatformOperation {
            operation_id: operation,
        }),
    )
    .await;
    let worker = worker(&database, &blobs);
    let run_state = || async {
        sqlx::query_scalar::<_, String>(
            "SELECT state FROM chatgpt_archive.import_runs WHERE export_id = $1",
        )
        .bind(received.export)
        .fetch_one(database.pool())
        .await
        .expect("the run is readable")
    };

    let fault = StagingFault::inject(root.path());
    let faulted = worker.process_pending_once().await;
    assert!(
        faulted.is_err(),
        "a storage fault is an error the pump retries, got {faulted:?}"
    );
    assert_ne!(
        run_state().await,
        "failed",
        "the run is not failed for good"
    );
    assert!(
        reports(&database, operation).await.is_empty(),
        "no failed report is queued for a fault that can heal"
    );

    fault.clear();
    drain(&worker).await;
    assert_eq!(run_state().await, "completed", "the next pass imports it");
    let reported = reports(&database, operation).await;
    assert_eq!(reported.len(), 1);
    assert_eq!(reported[0]["payload"]["status"], "succeeded");
    database.discard().await;
}
