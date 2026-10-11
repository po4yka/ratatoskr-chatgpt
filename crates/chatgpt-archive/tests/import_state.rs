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
