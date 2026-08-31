#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Process boundary for Ratatoskr `ChatGPT` Archive.
//!
//! Sequence, in this order and no other: load configuration, install
//! telemetry, connect the database when one is configured and apply the
//! schema, prepare the blob root, bind the operator listener, serve until
//! SIGTERM or SIGINT, drain within the configured bound, shut telemetry down,
//! exit 0.

mod lifecycle_commands;

pub use lifecycle_commands::main_result;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ratatoskr_chatgpt_archive::admin::{RuntimeState, admin_router};
use ratatoskr_chatgpt_archive::config::Config;
use ratatoskr_chatgpt_archive::fixture_admission::FixtureAdmissionError;
use ratatoskr_chatgpt_archive::parser_migration::ParserMigrationError;
use ratatoskr_chatgpt_archive::persistence::{Database, PersistenceError};
use ratatoskr_chatgpt_archive::portable_export::{
    PortableArchiveExporter, PortableExportError, PortableExportFilter,
};
use ratatoskr_chatgpt_archive::privacy_deletion::{PrivacyDeletionError, PrivacyDeletionScope};
use ratatoskr_chatgpt_archive::receipt::OperationReportOutbox;
use ratatoskr_chatgpt_archive::receipt::ReceiptError;
use ratatoskr_chatgpt_archive::receipt::auth::ConfigTenantAuthenticator;
use ratatoskr_chatgpt_archive::receipt::http::{ReceiptApiState, router as receipt_router};
use ratatoskr_chatgpt_archive::receipt::pg::PostgresReceiptRepository;
use ratatoskr_chatgpt_archive::reparse::ReparseError;
use ratatoskr_chatgpt_archive::{ArchiveReceiver, BlobStore, ParserId, init_telemetry};
use secrecy::ExposeSecret as _;
use uuid::Uuid;

/// Parsed `reparse` operator command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReparseCommand {
    /// Authenticated internal tenant identity.
    pub tenant_id: Uuid,
    /// Stable archive identity.
    pub archive_id: Uuid,
    /// Exact compiled parser identity.
    pub parser: ratatoskr_chatgpt_archive::ParserId,
    /// Whether persistence must be skipped.
    pub dry_run: bool,
}

/// Parsed `parser-migrate` operator command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParserMigrateCommand {
    /// Authenticated internal tenant identity.
    pub tenant_id: Uuid,
    /// Exact target parser identity.
    pub parser: ParserId,
    /// Whether execution must stop before persistence.
    pub dry_run: bool,
}

/// Parser migration command-line parsing failure.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParserMigrateCommandError {
    /// The supplied invocation is incomplete or malformed.
    #[error("parser-migrate arguments are invalid")]
    Invalid,
}

/// Parsed `fixture-admit` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureAdmitCommand {
    /// Candidate directory inspected without mutation.
    pub candidate: PathBuf,
}

/// Fixture admission command-line parsing failure.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FixtureAdmitCommandError {
    /// The supplied invocation is incomplete or malformed.
    #[error("fixture-admit arguments are invalid")]
    Invalid,
}

/// Parses arguments following the `fixture-admit` subcommand.
///
/// # Errors
///
/// Returns [`FixtureAdmitCommandError`] unless exactly one candidate path is
/// supplied.
pub fn parse_fixture_admit_command<I, S>(
    arguments: I,
) -> Result<FixtureAdmitCommand, FixtureAdmitCommandError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut arguments = arguments
        .into_iter()
        .map(|argument| argument.as_ref().to_owned());
    if arguments.next().as_deref() != Some("--candidate") {
        return Err(FixtureAdmitCommandError::Invalid);
    }
    let candidate = arguments
        .next()
        .filter(|value| !value.starts_with("--"))
        .ok_or(FixtureAdmitCommandError::Invalid)?;
    if arguments.next().is_some() {
        return Err(FixtureAdmitCommandError::Invalid);
    }
    Ok(FixtureAdmitCommand {
        candidate: PathBuf::from(candidate),
    })
}

/// Parses arguments following the `parser-migrate` subcommand.
///
/// # Errors
///
/// Returns [`ParserMigrateCommandError`] for missing, duplicate, or malformed
/// arguments.
pub fn parse_parser_migrate_command<I, S>(
    arguments: I,
) -> Result<ParserMigrateCommand, ParserMigrateCommandError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut arguments = arguments
        .into_iter()
        .map(|argument| argument.as_ref().to_owned());
    let mut tenant = None;
    let mut parser = None;
    let mut dry_run = false;
    while let Some(flag) = arguments.next() {
        match flag.as_str() {
            "--tenant" => set_migrate_once(&mut tenant, next_migrate_value(&mut arguments)?)?,
            "--parser" => set_migrate_once(&mut parser, next_migrate_value(&mut arguments)?)?,
            "--dry-run" if !dry_run => dry_run = true,
            _ => return Err(ParserMigrateCommandError::Invalid),
        }
    }
    let tenant_id = tenant
        .and_then(|value| Uuid::parse_str(&value).ok())
        .ok_or(ParserMigrateCommandError::Invalid)?;
    let parser = parser
        .ok_or(ParserMigrateCommandError::Invalid)
        .and_then(|value| {
            parse_parser_id(&value).map_err(|_| ParserMigrateCommandError::Invalid)
        })?;
    Ok(ParserMigrateCommand {
        tenant_id,
        parser,
        dry_run,
    })
}

fn next_migrate_value(
    arguments: &mut impl Iterator<Item = String>,
) -> Result<String, ParserMigrateCommandError> {
    arguments
        .next()
        .filter(|value| !value.starts_with("--"))
        .ok_or(ParserMigrateCommandError::Invalid)
}

fn set_migrate_once(
    target: &mut Option<String>,
    value: String,
) -> Result<(), ParserMigrateCommandError> {
    if target.replace(value).is_some() {
        return Err(ParserMigrateCommandError::Invalid);
    }
    Ok(())
}

/// Reparse command-line parsing failure.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReparseCommandError {
    /// The supplied invocation is incomplete or malformed.
    #[error("reparse arguments are invalid")]
    Invalid,
}

/// Parses arguments following the `reparse` subcommand.
///
/// # Errors
///
/// Returns [`ReparseCommandError`] for missing, duplicate, or malformed
/// arguments.
pub fn parse_reparse_command<I, S>(arguments: I) -> Result<ReparseCommand, ReparseCommandError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut arguments = arguments
        .into_iter()
        .map(|argument| argument.as_ref().to_owned());
    let mut tenant = None;
    let mut archive = None;
    let mut parser = None;
    let mut dry_run = false;
    while let Some(flag) = arguments.next() {
        match flag.as_str() {
            "--tenant" => set_reparse_once(&mut tenant, next_reparse_value(&mut arguments)?)?,
            "--archive" => set_reparse_once(&mut archive, next_reparse_value(&mut arguments)?)?,
            "--parser" => set_reparse_once(&mut parser, next_reparse_value(&mut arguments)?)?,
            "--dry-run" if !dry_run => dry_run = true,
            _ => return Err(ReparseCommandError::Invalid),
        }
    }
    let parser_value = parser.ok_or(ReparseCommandError::Invalid)?;
    let parser = parse_parser_id(&parser_value)?;
    Ok(ReparseCommand {
        tenant_id: parse_reparse_uuid(tenant)?,
        archive_id: parse_reparse_uuid(archive)?,
        parser,
        dry_run,
    })
}

fn next_reparse_value(
    arguments: &mut impl Iterator<Item = String>,
) -> Result<String, ReparseCommandError> {
    arguments
        .next()
        .filter(|value| !value.starts_with("--"))
        .ok_or(ReparseCommandError::Invalid)
}

fn set_reparse_once(target: &mut Option<String>, value: String) -> Result<(), ReparseCommandError> {
    if target.replace(value).is_some() {
        return Err(ReparseCommandError::Invalid);
    }
    Ok(())
}

fn parse_reparse_uuid(value: Option<String>) -> Result<Uuid, ReparseCommandError> {
    value
        .and_then(|value| Uuid::parse_str(&value).ok())
        .ok_or(ReparseCommandError::Invalid)
}

fn parse_parser_id(value: &str) -> Result<ParserId, ReparseCommandError> {
    let mut parts = value.split('@');
    let name = parts.next().unwrap_or_default();
    let version = parts.next().unwrap_or_default();
    if name.is_empty() || version.is_empty() || parts.next().is_some() {
        return Err(ReparseCommandError::Invalid);
    }
    Ok(ParserId {
        name: name.to_owned(),
        version: version.to_owned(),
    })
}

/// Parsed privacy deletion operator command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrivacyDeleteCommand {
    /// Persist a content-free preflight inventory.
    Plan {
        /// Authenticated internal tenant identity.
        tenant_id: Uuid,
        /// Stable request identity.
        request_id: Uuid,
        /// Exactly one archive, conversation, or tenant scope.
        scope: ratatoskr_chatgpt_archive::privacy_deletion::PrivacyDeletionScope,
    },
    /// Execute an already persisted plan.
    Execute {
        /// Authenticated internal tenant identity.
        tenant_id: Uuid,
        /// Stable request identity.
        request_id: Uuid,
    },
}

/// Privacy command-line parsing failure.
#[derive(Debug, thiserror::Error)]
pub enum PrivacyDeleteCommandError {
    /// The supplied invocation is incomplete, ambiguous, or unconfirmed.
    #[error("privacy-delete arguments are invalid")]
    Invalid,
}

/// Parses arguments following the `privacy-delete` subcommand.
///
/// # Errors
///
/// Returns [`PrivacyDeleteCommandError`] for missing, duplicate, ambiguous,
/// or unconfirmed arguments.
pub fn parse_privacy_delete_command<I, S>(
    arguments: I,
) -> Result<PrivacyDeleteCommand, PrivacyDeleteCommandError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut arguments = arguments
        .into_iter()
        .map(|argument| argument.as_ref().to_owned());
    let operation = arguments.next().ok_or(PrivacyDeleteCommandError::Invalid)?;
    let mut tenant = None;
    let mut request = None;
    let mut archive = None;
    let mut conversation = None;
    let mut all = false;
    let mut confirm = false;
    while let Some(flag) = arguments.next() {
        match flag.as_str() {
            "--tenant" => set_once(&mut tenant, next_value(&mut arguments)?)?,
            "--request" => set_once(&mut request, next_value(&mut arguments)?)?,
            "--archive" => set_once(&mut archive, next_value(&mut arguments)?)?,
            "--conversation" => set_once(&mut conversation, next_value(&mut arguments)?)?,
            "--all" if !all => all = true,
            "--confirm" if !confirm => confirm = true,
            _ => return Err(PrivacyDeleteCommandError::Invalid),
        }
    }
    let tenant_id = parse_uuid(tenant)?;
    let request_id = parse_uuid(request)?;
    match operation.as_str() {
        "plan" if !confirm => {
            let selected = usize::from(archive.is_some())
                + usize::from(conversation.is_some())
                + usize::from(all);
            if selected != 1 {
                return Err(PrivacyDeleteCommandError::Invalid);
            }
            let scope = if let Some(value) = archive {
                PrivacyDeletionScope::Archive {
                    ai_archive_id: parse_uuid(Some(value))?,
                }
            } else if let Some(value) = conversation {
                PrivacyDeletionScope::Conversation {
                    conversation_id: parse_uuid(Some(value))?,
                }
            } else {
                PrivacyDeletionScope::Tenant
            };
            Ok(PrivacyDeleteCommand::Plan {
                tenant_id,
                request_id,
                scope,
            })
        }
        "execute" if confirm && archive.is_none() && conversation.is_none() && !all => {
            Ok(PrivacyDeleteCommand::Execute {
                tenant_id,
                request_id,
            })
        }
        _ => Err(PrivacyDeleteCommandError::Invalid),
    }
}

fn next_value(
    arguments: &mut impl Iterator<Item = String>,
) -> Result<String, PrivacyDeleteCommandError> {
    arguments
        .next()
        .filter(|value| !value.starts_with("--"))
        .ok_or(PrivacyDeleteCommandError::Invalid)
}

fn set_once(target: &mut Option<String>, value: String) -> Result<(), PrivacyDeleteCommandError> {
    if target.replace(value).is_some() {
        return Err(PrivacyDeleteCommandError::Invalid);
    }
    Ok(())
}

fn parse_uuid(value: Option<String>) -> Result<Uuid, PrivacyDeleteCommandError> {
    value
        .and_then(|value| Uuid::parse_str(&value).ok())
        .ok_or(PrivacyDeleteCommandError::Invalid)
}

/// Parsed `portable-export` command arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableExportCommand {
    /// Required tenant and optional record filters.
    pub filter: PortableExportFilter,
    /// Destination ZIP path.
    pub output: PathBuf,
}

/// Command-line parsing failure.
#[derive(Debug, thiserror::Error)]
pub enum PortableExportCommandError {
    /// The supplied invocation is incomplete or malformed.
    #[error("portable-export arguments are invalid")]
    Invalid,
}

/// Parses portable-export arguments after the subcommand name.
///
/// # Errors
///
/// Returns [`PortableExportCommandError`] when required arguments are absent.
pub fn parse_portable_export_command<I, S>(
    arguments: I,
) -> Result<PortableExportCommand, PortableExportCommandError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut arguments = arguments
        .into_iter()
        .map(|argument| argument.as_ref().to_owned());
    let mut tenant = None;
    let mut output = None;
    let mut project = None;
    let mut observed_from = None;
    let mut observed_to = None;
    while let Some(flag) = arguments.next() {
        let value = arguments
            .next()
            .ok_or(PortableExportCommandError::Invalid)?;
        match flag.as_str() {
            "--tenant" => tenant = Some(value),
            "--output" => output = Some(PathBuf::from(value)),
            "--project" => project = Some(value),
            "--from" => observed_from = Some(value),
            "--to" => observed_to = Some(value),
            _ => return Err(PortableExportCommandError::Invalid),
        }
    }
    Ok(PortableExportCommand {
        filter: PortableExportFilter {
            account_external_ref: tenant.ok_or(PortableExportCommandError::Invalid)?,
            project_external_id: project,
            observed_from_rfc3339: observed_from,
            observed_to_rfc3339: observed_to,
        },
        output: output.ok_or(PortableExportCommandError::Invalid)?,
    })
}

/// A failure that prevents the process from serving.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ServiceError {
    /// Telemetry refused installation.
    #[error("telemetry failed")]
    Telemetry(#[from] ratatoskr_chatgpt_archive::TelemetryError),
    /// The database was configured but unusable.
    #[error("database failed")]
    Persistence(#[from] PersistenceError),
    /// A listener could not bind.
    #[error("the admin listener could not bind")]
    Bind(#[source] std::io::Error),
    /// Serving failed.
    #[error("serving failed")]
    Serve(#[source] std::io::Error),
    /// The blob storage root was unusable.
    #[error("blob storage failed")]
    BlobStore(#[from] ratatoskr_chatgpt_archive::BlobStoreError),
    /// Archive receipt could not anchor its staging or storage.
    #[error("archive receipt failed")]
    Receipt(#[from] ReceiptError),
    /// The blob storage root is not configured. Unreachable through the real
    /// loader: configuration refuses to start without one.
    #[error("the blob storage root is not configured")]
    MissingBlobRoot,
    /// Portable export could not read or publish the selected evidence.
    #[error("portable export failed")]
    PortableExport(#[from] PortableExportError),
    /// Privacy deletion planning or execution failed.
    #[error("privacy deletion failed")]
    PrivacyDeletion(#[from] PrivacyDeletionError),
    /// Reparse planning or execution failed.
    #[error("reparse failed")]
    Reparse(#[from] ReparseError),
    /// Parser migration planning or report persistence failed.
    #[error("parser migration failed")]
    ParserMigration(#[from] ParserMigrationError),
    /// Fixture candidate inspection failed before a report could be produced.
    #[error("fixture admission failed")]
    FixtureAdmission(#[from] FixtureAdmissionError),
    /// The compiled runtime parser set was internally inconsistent.
    #[error("runtime parser registry failed")]
    ParserRegistry(#[from] ratatoskr_chatgpt_archive::RegistryError),
}

/// Resolves when SIGINT or SIGTERM arrives.
///
/// # Panics
///
/// Never reaches a request path: handler installation failing at startup is a
/// programming error, so it is a startup panic rather than an error value.
#[allow(
    clippy::expect_used,
    reason = "signal-handler installation can only fail through a programming error; \
              there is no meaningful refusal path for a process that cannot be stopped"
)]
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("installing the SIGINT handler must work");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("installing the SIGTERM handler must work")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

/// Runs bounded publication passes until the owning service starts shutdown.
async fn operation_report_loop(
    outbox: OperationReportOutbox,
    endpoint: String,
    nkey_seed_path: std::path::PathBuf,
    ready: Arc<std::sync::atomic::AtomicBool>,
    mut stopped: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        if *stopped.borrow() {
            return;
        }
        let pass_ready = outbox
            .publish_pending_once(&endpoint, &nkey_seed_path)
            .await
            .is_ok();
        ready.store(pass_ready, std::sync::atomic::Ordering::Release);
        if !pass_ready {
            metrics::counter!("chatgpt_archive_operation_report_publications_total", "outcome" => "failed")
                .increment(1);
        }
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(2)) => {},
            changed = stopped.changed() => {
                if changed.is_err() || *stopped.borrow() {
                    return;
                }
            }
        }
    }
}

async fn initial_import_loop(
    worker: ratatoskr_chatgpt_archive::InitialImportWorker,
    ready: Arc<std::sync::atomic::AtomicBool>,
    mut stopped: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        if *stopped.borrow() {
            return;
        }
        let pass_ready = worker.process_pending_once().await.is_ok();
        ready.store(pass_ready, std::sync::atomic::Ordering::Release);
        if !pass_ready {
            metrics::counter!("chatgpt_archive_import_passes_total", "outcome" => "failed")
                .increment(1);
        }
        tokio::select! {
            () = tokio::time::sleep(Duration::from_millis(250)) => {},
            changed = stopped.changed() => {
                if changed.is_err() || *stopped.borrow() {
                    return;
                }
            }
        }
    }
}

fn register_flag_check(
    state: &RuntimeState,
    name: &str,
    ready: Arc<std::sync::atomic::AtomicBool>,
    unavailable: &'static str,
) {
    state.register_check(name, move || {
        let ready = Arc::clone(&ready);
        async move {
            ready
                .load(std::sync::atomic::Ordering::Acquire)
                .then_some(())
                .ok_or_else(|| unavailable.to_owned())
        }
    });
}

fn start_initial_import_worker(
    database: &Database,
    blob: &BlobStore,
    limits: &ratatoskr_chatgpt_archive::Limits,
    state: &RuntimeState,
    stopped: tokio::sync::watch::Receiver<bool>,
) -> Result<tokio::task::JoinHandle<()>, ServiceError> {
    let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
    register_flag_check(
        state,
        "initial_import_worker",
        Arc::clone(&ready),
        "initial import worker unavailable",
    );
    let registry = Arc::new(ratatoskr_chatgpt_archive::ParserRegistry::runtime()?);
    let worker = ratatoskr_chatgpt_archive::InitialImportWorker::new(
        database.pool().clone(),
        blob.clone(),
        registry,
        limits.into(),
    );
    Ok(tokio::spawn(initial_import_loop(worker, ready, stopped)))
}

fn start_operation_reporter(
    database: &Database,
    config: &Config,
    state: &RuntimeState,
    stopped: tokio::sync::watch::Receiver<bool>,
) -> Option<tokio::task::JoinHandle<()>> {
    let (endpoint, nkey_seed_path) = (
        config.receipt.event_bus_url.as_ref()?,
        config.receipt.event_bus_nkey_seed_path.as_ref()?,
    );
    let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
    register_flag_check(
        state,
        "operation_report_publisher",
        Arc::clone(&ready),
        "operation report publisher unavailable",
    );
    Some(tokio::spawn(operation_report_loop(
        OperationReportOutbox::new(database.pool().clone()),
        endpoint.expose_secret().to_owned(),
        nkey_seed_path.clone(),
        ready,
        stopped,
    )))
}

/// Runs the service to completion.
///
/// # Errors
///
/// Returns [`ServiceError`] when any startup step fails or serving ends with
/// an I/O error.
pub async fn run(config: &Config) -> Result<(), ServiceError> {
    // Telemetry first: every later step logs through it.
    let guard = init_telemetry(&config.telemetry)?;
    let metrics_handle = guard.metrics_handle();

    tracing::info!(
        service = ratatoskr_chatgpt_archive::telemetry::SERVICE_NAME,
        version = ratatoskr_chatgpt_archive::telemetry::VERSION,
        "starting"
    );

    let state = Arc::new(RuntimeState::new());
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let mut outbox_task = None;
    let mut import_task = None;

    // Prepare the archive root before anything can write into it.
    let root = config
        .storage
        .blob_root
        .clone()
        .ok_or(ServiceError::MissingBlobRoot)?;
    let blob = BlobStore::new(&root)?;

    // Durable receipt needs a database and a staging location; without
    // either, the public surface simply does not mount and the admin plane
    // serves alone.
    let mut public_routes = None;
    if config.storage.database_url.is_some() {
        let database = Arc::new(Database::connect(&config.storage, &config.limits).await?);
        database.apply_schema().await?;
        {
            let probe = Arc::clone(&database);
            state.register_check("database", move || {
                let probe = Arc::clone(&probe);
                async move { probe.ping().await.map_err(|error| error.to_string()) }
            });
        }

        if let Some(staging_root) = config.storage.receipt_staging_root.clone() {
            let repository = PostgresReceiptRepository::new(database.pool().clone());
            let receiver = ArchiveReceiver::new(
                blob.clone(),
                Arc::new(repository),
                staging_root,
                config.limits.max_archive_bytes,
            )?;
            let swept = receiver.sweep_interrupted().await;
            if swept > 0 {
                tracing::info!(runs = swept, "swept interrupted import runs");
            }
            let authenticator = Arc::new(ConfigTenantAuthenticator::from_config(&config.receipt));
            public_routes = Some(receipt_router(Arc::new(
                ReceiptApiState::new_with_platform_account_ids(
                    receiver,
                    authenticator,
                    &config.receipt.platform_accounts,
                ),
            )));

            import_task = Some(start_initial_import_worker(
                &database,
                &blob,
                &config.limits,
                &state,
                shutdown_rx.clone(),
            )?);
        }

        outbox_task = start_operation_reporter(&database, config, &state, shutdown_rx.clone());
    }

    let listener = tokio::net::TcpListener::bind(config.admin.listen_address)
        .await
        .map_err(ServiceError::Bind)?;

    let receipt_served = public_routes.is_some();
    let mut app = admin_router(Arc::clone(&state), move || metrics_handle.render());
    if let Some(public) = public_routes {
        app = app.merge(public);
    }

    tracing::info!(
        address = %config.admin.listen_address,
        receipt_served,
        "listening"
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            let _ = shutdown_tx.send(true);
        })
        .await
        .map_err(ServiceError::Serve)?;

    if let Some(task) = outbox_task {
        let _ = tokio::time::timeout(
            Duration::from_millis(config.limits.shutdown_timeout_ms),
            task,
        )
        .await;
    }
    if let Some(task) = import_task {
        let _ = tokio::time::timeout(
            Duration::from_millis(config.limits.shutdown_timeout_ms),
            task,
        )
        .await;
    }

    Ok(())
}

/// Exports one configured tenant selection without starting the HTTP service.
///
/// # Errors
///
/// Returns [`ServiceError`] when storage, persistence, or output publication
/// fails.
pub async fn run_portable_export(
    config: &Config,
    command: &PortableExportCommand,
) -> Result<(), ServiceError> {
    let root = config
        .storage
        .blob_root
        .clone()
        .ok_or(ServiceError::MissingBlobRoot)?;
    let database = Database::connect(&config.storage, &config.limits).await?;
    database.apply_schema().await?;
    let state = database
        .load_portable_archive_state(&command.filter)
        .await?;
    let blob_store = BlobStore::new(&root)?;
    PortableArchiveExporter::new()
        .export_to_path_with_assets(&state, &blob_store, &command.output)
        .await?;
    Ok(())
}
