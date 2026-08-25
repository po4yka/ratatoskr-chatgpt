#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Process boundary for Ratatoskr `ChatGPT` Archive.
//!
//! Sequence, in this order and no other: load configuration, install
//! telemetry, connect the database when one is configured and apply the
//! schema, prepare the blob root, bind the operator listener, serve until
//! SIGTERM or SIGINT, drain within the configured bound, shut telemetry down,
//! exit 0.

use std::process::ExitCode;
use std::sync::Arc;

use ratatoskr_chatgpt_archive::admin::{RuntimeState, admin_router};
use ratatoskr_chatgpt_archive::config::Config;
use ratatoskr_chatgpt_archive::persistence::Database;
use ratatoskr_chatgpt_archive::{BlobStore, init_telemetry};

/// A failure that prevents the process from serving.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ServiceError {
    /// Telemetry refused installation.
    #[error("telemetry failed")]
    Telemetry(#[from] ratatoskr_chatgpt_archive::TelemetryError),
    /// The database was configured but unusable.
    #[error("database failed")]
    Persistence(#[from] ratatoskr_chatgpt_archive::PersistenceError),
    /// A listener could not bind.
    #[error("the admin listener could not bind")]
    Bind(#[source] std::io::Error),
    /// Serving failed.
    #[error("serving failed")]
    Serve(#[source] std::io::Error),
    /// The blob storage root was unusable.
    #[error("blob storage failed")]
    BlobStore(#[from] ratatoskr_chatgpt_archive::BlobStoreError),
    /// The blob storage root is not configured. Unreachable through the real
    /// loader: configuration refuses to start without one.
    #[error("the blob storage root is not configured")]
    MissingBlobRoot,
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

    // The database is optional at this milestone: without a URL there is one
    // fewer readiness check, not a refusal.
    if config.storage.database_url.is_some() {
        let database = Database::connect(&config.storage, &config.limits).await?;
        database.apply_schema().await?;
        let probe = Arc::new(database);
        state.register_check("database", move || {
            let probe = Arc::clone(&probe);
            async move { probe.ping().await.map_err(|error| error.to_string()) }
        });
    }

    // Prepare the archive root before anything can write into it.
    let root = config
        .storage
        .blob_root
        .clone()
        .ok_or(ServiceError::MissingBlobRoot)?;
    BlobStore::new(&root)?;

    let listener = tokio::net::TcpListener::bind(config.admin.listen_address)
        .await
        .map_err(ServiceError::Bind)?;

    let app = admin_router(state, move || metrics_handle.render());

    tracing::info!(
        address = %config.admin.listen_address,
        "listening"
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(ServiceError::Serve)?;

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
