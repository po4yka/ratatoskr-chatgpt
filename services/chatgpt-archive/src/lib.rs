#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Process boundary for Ratatoskr `ChatGPT` Archive.
//!
//! Sequence, in this order and no other: load configuration, install
//! telemetry, connect the database when one is configured and apply the
//! schema, prepare the blob root, bind the operator listener, serve until
//! SIGTERM or SIGINT, drain within the configured bound, shut telemetry down,
//! exit 0.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use ratatoskr_chatgpt_archive::admin::{RuntimeState, admin_router};
use ratatoskr_chatgpt_archive::config::Config;
use ratatoskr_chatgpt_archive::persistence::{Database, PersistenceError};
use ratatoskr_chatgpt_archive::portable_export::{
    PortableArchiveExporter, PortableExportError, PortableExportFilter,
};
use ratatoskr_chatgpt_archive::receipt::OperationReportOutbox;
use ratatoskr_chatgpt_archive::receipt::ReceiptError;
use ratatoskr_chatgpt_archive::receipt::auth::ConfigTenantAuthenticator;
use ratatoskr_chatgpt_archive::receipt::http::{ReceiptApiState, router as receipt_router};
use ratatoskr_chatgpt_archive::receipt::pg::PostgresReceiptRepository;
use ratatoskr_chatgpt_archive::{ArchiveReceiver, BlobStore, init_telemetry};
use secrecy::ExposeSecret as _;

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
    mut stopped: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        if *stopped.borrow() {
            return;
        }
        if outbox.publish_pending_once(&endpoint).await.is_err() {
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
                blob,
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
        }

        if let Some(endpoint) = config.receipt.event_bus_url.as_ref() {
            let endpoint = endpoint.expose_secret().to_owned();
            let outbox = OperationReportOutbox::new(database.pool().clone());
            let stopped = shutdown_rx.clone();
            outbox_task = Some(tokio::spawn(operation_report_loop(
                outbox, endpoint, stopped,
            )));
        }
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

/// The process entry point: configuration failures exit 78 (`EX_CONFIG`), other
/// startup failures exit 1 with a value-free stderr line.
///
/// # Panics
///
/// Never reaches a request path: a process whose async runtime cannot be built
/// cannot even reach configuration loading.
#[must_use]
#[allow(
    clippy::expect_used,
    reason = "runtime construction can only fail through an invalid builder; there is no \
              meaningful refusal path for a process that cannot schedule at all"
)]
pub fn main_result() -> ExitCode {
    let mut arguments = std::env::args();
    let _program = arguments.next();
    let subcommand = arguments.next();
    if subcommand.as_deref() == Some("portable-export") {
        let Ok(command) = parse_portable_export_command(arguments) else {
            eprintln!("ratatoskr-chatgpt-archive: portable-export requires --tenant and --output");
            return ExitCode::from(64);
        };
        return portable_export_main(&command);
    }
    let config = match Config::load() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("ratatoskr-chatgpt-archive: {error}");
            return ExitCode::from(78);
        }
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("building the async runtime must work");
    let outcome = runtime.block_on(run(&config));
    runtime.shutdown_timeout(std::time::Duration::from_millis(
        config.limits.shutdown_timeout_ms,
    ));

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(chain = %error, "the service stopped");
            eprintln!("ratatoskr-chatgpt-archive: the service stopped");
            ExitCode::FAILURE
        }
    }
}

#[allow(
    clippy::expect_used,
    reason = "runtime construction can only fail through an invalid builder"
)]
fn portable_export_main(command: &PortableExportCommand) -> ExitCode {
    let config = match Config::load() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("ratatoskr-chatgpt-archive: {error}");
            return ExitCode::from(78);
        }
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("building the async runtime must work");
    let outcome = runtime.block_on(run_portable_export(&config, command));
    runtime.shutdown_timeout(std::time::Duration::from_millis(
        config.limits.shutdown_timeout_ms,
    ));
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(chain = %error, "portable export failed");
            eprintln!("ratatoskr-chatgpt-archive: portable export failed");
            ExitCode::FAILURE
        }
    }
}
