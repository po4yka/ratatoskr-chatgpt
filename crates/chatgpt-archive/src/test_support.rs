//! Shared hand-written test doubles.
//!
//! Compiled only under the `test-support` feature, which the crate's own
//! dev-dependencies enable. No mocking crates: every double is readable in
//! one screen and records what it was asked.
//!
//! The state lives behind one `Arc` handle so each returned future owns its
//! state and the `'static` seam bound holds without lifetime plumbing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use uuid::Uuid;

use crate::receipt::AcquisitionMode;
use crate::receipt::report::raw_stored_partial;
use crate::receipt::repository::{
    PublishRequest, PublishedExport, ReceiptRepository, RepoFuture, RepositoryError, RunRecord,
};
use crate::receipt::state::ImportState;

/// The backend failure the fake reports when a lock is poisoned or a run id
/// is unknown.
fn backend(message: &'static str) -> RepositoryError {
    RepositoryError::backend(std::io::Error::other(message))
}

/// One run as the fake records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeRun {
    /// Owning tenant reference.
    pub account_external_ref: String,
    /// Acquisition mode spelling.
    pub acquisition_mode: String,
    /// Media type declared at receipt start.
    pub media_type: String,
    /// Current machine stage.
    pub state: ImportState,
    /// Digest captured at `hashed`.
    pub sha256_hex: Option<String>,
    /// Length captured at `hashed`.
    pub byte_length: Option<i64>,
    /// Export produced at `stored`.
    pub export_id: Option<Uuid>,
}

impl FakeRun {
    fn base(account_external_ref: &str) -> Self {
        Self {
            account_external_ref: account_external_ref.to_owned(),
            acquisition_mode: "consumer_export".to_owned(),
            media_type: "application/zip".to_owned(),
            state: ImportState::Received,
            sha256_hex: None,
            byte_length: None,
            export_id: None,
        }
    }

    /// A run sitting at `received` for this tenant.
    #[must_use]
    pub fn received(account_external_ref: &str) -> Self {
        Self::base(account_external_ref)
    }

    /// A run sitting at `hashed` carrying its digest evidence.
    #[must_use]
    pub fn hashed(account_external_ref: &str, sha256_hex: &str, byte_length: u64) -> Self {
        let mut run = Self::base(account_external_ref);
        run.state = ImportState::Hashed;
        run.sha256_hex = Some(sha256_hex.to_owned());
        run.byte_length = Some(i64::try_from(byte_length).unwrap_or(i64::MAX));
        run
    }
}

/// One export the fake recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeExport {
    /// The export identity.
    pub export_id: Uuid,
    /// Ratatoskr archive-import identity.
    pub ai_archive_id: Uuid,
    /// Owning tenant reference.
    pub account_external_ref: String,
    /// Digest of the evidence.
    pub sha256_hex: String,
    /// Blob reference JSON as handed over by the receiver.
    pub blob_ref_json: serde_json::Value,
}

#[derive(Debug, Default)]
struct State {
    runs: HashMap<Uuid, FakeRun>,
    exports: Vec<FakeExport>,
    advances: Vec<(Uuid, ImportState, ImportState)>,
    publishes: Vec<(Uuid, String, String, u64)>,
    operation_reports: Vec<serde_json::Value>,
}

/// An in-memory [`ReceiptRepository`] for receiver tests.
#[derive(Debug, Default)]
pub struct FakeReceiptRepository {
    state: Arc<Mutex<State>>,
}

impl FakeReceiptRepository {
    /// A fresh empty repository.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        // A poisoned fake means an earlier test panicked mid-call; failing
        // the current test loudly is exactly what a test double should do.
        #[allow(
            clippy::expect_used,
            reason = "test double: poisoning must surface as a test failure"
        )]
        {
            self.state.lock().expect("fake state poisoned")
        }
    }

    /// Snapshots the recorded exports.
    #[must_use]
    pub fn exports_snapshot(&self) -> Vec<FakeExport> {
        self.lock().exports.clone()
    }

    /// Snapshots the recorded runs with their identities.
    #[must_use]
    pub fn runs_snapshot(&self) -> Vec<(Uuid, FakeRun)> {
        self.lock()
            .runs
            .iter()
            .map(|(id, run)| (*id, run.clone()))
            .collect()
    }

    /// Snapshots the publish attempts as `(run, tenant, digest, length)`.
    #[must_use]
    pub fn publishes(&self) -> Vec<(Uuid, String, String, u64)> {
        self.lock().publishes.clone()
    }

    /// Snapshots the terminal operation reports recorded for Platform receipts.
    #[must_use]
    pub fn operation_reports(&self) -> Vec<serde_json::Value> {
        self.lock().operation_reports.clone()
    }

    /// Loads one run's fake record.
    #[must_use]
    pub fn run(&self, run_id: Uuid) -> Option<FakeRun> {
        self.lock().runs.get(&run_id).cloned()
    }

    /// Pre-seeds a run record, simulating a crash left behind; returns its
    /// freshly minted identity.
    #[must_use]
    pub fn seed_run(&self, run: FakeRun) -> Uuid {
        let id = Uuid::now_v7();
        self.lock().runs.insert(id, run);
        id
    }
}

impl ReceiptRepository for FakeReceiptRepository {
    fn create_run(
        &self,
        account_external_ref: &str,
        mode: &AcquisitionMode,
        media_type: &str,
    ) -> RepoFuture<Result<Uuid, RepositoryError>> {
        let state = Arc::clone(&self.state);
        let mut record = FakeRun::base(account_external_ref);
        mode.as_str().clone_into(&mut record.acquisition_mode);
        media_type.clone_into(&mut record.media_type);
        Box::pin(async move {
            let id = Uuid::now_v7();
            state
                .lock()
                .map_err(|_| backend("runs poisoned"))?
                .runs
                .insert(id, record);
            Ok(id)
        })
    }

    fn record_hash(
        &self,
        run_id: Uuid,
        sha256_hex: String,
        byte_length: u64,
    ) -> RepoFuture<Result<(), RepositoryError>> {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            let mut guard = state.lock().map_err(|_| backend("runs poisoned"))?;
            let run = guard
                .runs
                .get_mut(&run_id)
                .ok_or_else(|| backend("no such run"))?;
            if run.state != ImportState::Received {
                return Err(RepositoryError::Conflict);
            }
            run.state = ImportState::Hashed;
            run.sha256_hex = Some(sha256_hex);
            run.byte_length = Some(i64::try_from(byte_length).unwrap_or(i64::MAX));
            Ok(())
        })
    }

    fn mark_run(
        &self,
        run_id: Uuid,
        expected: &ImportState,
        target: ImportState,
    ) -> RepoFuture<Result<(), RepositoryError>> {
        let state = Arc::clone(&self.state);
        let expected = expected.clone();
        Box::pin(async move {
            let mut guard = state.lock().map_err(|_| backend("runs poisoned"))?;
            guard
                .advances
                .push((run_id, expected.clone(), target.clone()));
            let run = guard
                .runs
                .get_mut(&run_id)
                .ok_or_else(|| backend("no such run"))?;
            if run.state != expected {
                return Err(RepositoryError::Conflict);
            }
            run.state = target;
            Ok(())
        })
    }

    fn load_run(&self, run_id: Uuid) -> RepoFuture<Result<Option<RunRecord>, RepositoryError>> {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            let guard = state.lock().map_err(|_| backend("runs poisoned"))?;
            Ok(guard.runs.get(&run_id).map(|run| RunRecord {
                id: run_id,
                account_external_ref: run.account_external_ref.clone(),
                acquisition_mode: run.acquisition_mode.clone(),
                media_type: run.media_type.clone(),
                state: run.state.clone(),
                sha256_hex: run.sha256_hex.clone(),
                byte_length: run.byte_length,
                export_id: run.export_id,
            }))
        })
    }

    fn list_resumable(&self) -> RepoFuture<Result<Vec<RunRecord>, RepositoryError>> {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            let guard = state.lock().map_err(|_| backend("runs poisoned"))?;
            let mut records: Vec<(Uuid, &FakeRun)> = guard
                .runs
                .iter()
                .filter(|(_, run)| matches!(run.state, ImportState::Received | ImportState::Hashed))
                .map(|(id, run)| (*id, run))
                .collect();
            records.sort_by_key(|(id, _)| *id);
            Ok(records
                .into_iter()
                .map(|(id, run)| RunRecord {
                    id,
                    account_external_ref: run.account_external_ref.clone(),
                    acquisition_mode: run.acquisition_mode.clone(),
                    media_type: run.media_type.clone(),
                    state: run.state.clone(),
                    sha256_hex: run.sha256_hex.clone(),
                    byte_length: run.byte_length,
                    export_id: run.export_id,
                })
                .collect())
        })
    }

    fn find_export_by_digest(
        &self,
        account_external_ref: &str,
        sha256_hex: &str,
    ) -> RepoFuture<Result<Option<Uuid>, RepositoryError>> {
        let state = Arc::clone(&self.state);
        let account = account_external_ref.to_owned();
        let digest = sha256_hex.to_owned();
        Box::pin(async move {
            let guard = state.lock().map_err(|_| backend("exports poisoned"))?;
            Ok(guard
                .exports
                .iter()
                .find(|export| {
                    export.account_external_ref == account && export.sha256_hex == digest
                })
                .map(|found| found.export_id))
        })
    }

    fn publish_export(
        &self,
        request: PublishRequest,
    ) -> RepoFuture<Result<PublishedExport, RepositoryError>> {
        let state = Arc::clone(&self.state);
        let account = request.account_external_ref;
        let run_id = request.run_id;
        let blob_ref_json = request.blob_ref_json;
        let sha256_hex = request.sha256_hex;
        let byte_length = request.byte_length;
        let platform_operation = request.platform_operation;
        Box::pin(async move {
            let mut guard = state.lock().map_err(|_| backend("publishes poisoned"))?;
            guard
                .publishes
                .push((run_id, account.clone(), sha256_hex.clone(), byte_length));
            if let Some(existing) = guard.exports.iter().find_map(|export| {
                (export.account_external_ref == account && export.sha256_hex == sha256_hex)
                    .then_some(export.export_id)
            }) {
                return Err(RepositoryError::DuplicateExisting {
                    existing_export_id: existing,
                });
            }
            let export_id = Uuid::now_v7();
            let ai_archive_id = Uuid::now_v7();
            if let Some(operation) = platform_operation {
                guard
                    .operation_reports
                    .push(raw_stored_partial(operation, ai_archive_id)?);
            }
            guard.exports.push(FakeExport {
                export_id,
                ai_archive_id,
                account_external_ref: account,
                sha256_hex,
                blob_ref_json,
            });
            let run = guard
                .runs
                .get_mut(&run_id)
                .ok_or_else(|| backend("no such run"))?;
            run.state = ImportState::Stored;
            run.export_id = Some(export_id);
            Ok(PublishedExport {
                export_id,
                ai_archive_id,
            })
        })
    }
}
