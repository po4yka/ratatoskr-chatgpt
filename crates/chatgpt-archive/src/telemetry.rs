//! Structured telemetry: the JSON log pipeline and the Prometheus registry.
//!
//! Installed exactly once per process. A second installation attempt is a
//! refusal, not a reset: two subscribers or two recorders would split every
//! observation after startup.

use std::sync::{Arc, Mutex};

use metrics::gauge;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::config::TelemetryConfig;

/// The one wire identity of this bounded context.
pub const SERVICE_NAME: &str = "ratatoskr-chatgpt";

/// The deployable role this binary serves. One process, one role.
pub const ROLE: &str = "archive";

/// The crate version, compiled in.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The build's git commit, provided by the container build, or `unknown`
/// outside one — the first thing anyone checks when a deployment misbehaves.
pub const GIT_SHA: &str = match option_env!("RATATOSKR_GIT_SHA") {
    Some(sha) => sha,
    None => "unknown",
};

/// The declared toolchain.
pub const RUST_VERSION: &str = env!("CARGO_PKG_RUST_VERSION");

/// The build-identity gauge: one series, labelled with the compiled identity.
const BUILD_INFO_METRIC: &str = "chatgpt_build_info";

/// Telemetry bootstrap failure.
#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    /// The configured filter expression did not parse.
    #[error("telemetry could not be initialized: the log filter is invalid")]
    LogFilter(#[source] tracing_subscriber::filter::ParseError),
    /// The Prometheus recorder could not be installed.
    #[error("telemetry could not be initialized: the metrics recorder refused installation")]
    MetricsRecorder(#[source] metrics_exporter_prometheus::BuildError),
    /// A global subscriber is already installed; two subscribers would split
    /// every observation after startup.
    #[error("telemetry is already initialized")]
    AlreadyInstalled(#[source] tracing_subscriber::util::TryInitError),
}

/// Owns the telemetry runtime for the life of the process.
#[derive(Debug)]
pub struct TelemetryGuard {
    /// The text exposition renderer of the installed recorder.
    pub(crate) metrics_handle: PrometheusHandle,
}

impl TelemetryGuard {
    /// A cloneable renderer of the installed recorder, handed to whatever
    /// surface serves the exposition text.
    #[must_use]
    pub fn metrics_handle(&self) -> PrometheusHandle {
        self.metrics_handle.clone()
    }

    /// Releases telemetry resources before exit. Consuming `self` makes a
    /// second call unrepresentable; shutdown ordering belongs to the caller.
    pub fn shutdown(self) {}
}

/// Installs the process-wide structured telemetry once.
///
/// # Errors
///
/// Returns [`TelemetryError`] when the filter expression is invalid, a global
/// subscriber is already installed, or the Prometheus recorder cannot be
/// installed.
pub fn init_telemetry(config: &TelemetryConfig) -> Result<TelemetryGuard, TelemetryError> {
    let filter = EnvFilter::try_new(&config.log_filter).map_err(TelemetryError::LogFilter)?;

    let metrics_handle = PrometheusBuilder::new()
        .install_recorder()
        .map_err(TelemetryError::MetricsRecorder)?;

    json_subscriber(filter, std::io::stdout)
        .try_init()
        .map_err(|error| {
            // The recorder is already global at this point; nothing can uninstall
            // it, so the caller must treat this failure as fatal rather than retry
            // into a half-installed state.
            TelemetryError::AlreadyInstalled(error)
        })?;

    gauge!(BUILD_INFO_METRIC,
        "service" => SERVICE_NAME,
        "role" => ROLE,
        "version" => VERSION,
        "git_sha" => GIT_SHA,
        "rust_version" => RUST_VERSION,
    )
    .set(1.0);

    Ok(TelemetryGuard { metrics_handle })
}

/// Renders the startup identity record through the production JSON formatter
/// into a string.
///
/// Exists so contract tests can parse exactly what an operator's first log
/// line looks like without scraping the process's stdout. It touches no
/// global state: the subscriber is thread-local for the duration of one emit.
#[must_use]
pub fn render_startup_record() -> String {
    let buffer = RecordBuffer(Arc::new(Mutex::new(Vec::new())));
    emit_startup_record(json_subscriber(EnvFilter::new("info"), buffer.clone()));
    buffer.snapshot()
}

/// One JSON formatter configuration, shared by the global install and the
/// contract-test renderer so both produce byte-identical record shapes.
fn json_subscriber<W>(filter: EnvFilter, writer: W) -> impl tracing::Subscriber + Send + Sync
where
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .with_writer(writer)
        .finish()
}

/// Emits the startup identity record through `subscriber`.
fn emit_startup_record<S>(subscriber: S)
where
    S: tracing::Subscriber + Send + Sync + 'static,
{
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(
            service_name = SERVICE_NAME,
            version = VERSION,
            git_sha = GIT_SHA,
            "startup"
        );
    });
}

/// A shared in-memory writer capturing what the formatter produces.
#[derive(Clone)]
struct RecordBuffer(Arc<Mutex<Vec<u8>>>);

impl RecordBuffer {
    fn snapshot(&self) -> String {
        // A poisoned mutex means a writer panicked mid-record; the bytes
        // written so far are still the honest answer to render.
        let guard = match self.0.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        String::from_utf8_lossy(&guard).into_owned()
    }
}

struct RecordBufferWriter<'a>(&'a RecordBuffer);

impl std::io::Write for RecordBufferWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut guard = match self.0.0.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for RecordBuffer {
    type Writer = RecordBufferWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        RecordBufferWriter(self)
    }
}

#[cfg(test)]
mod tests {
    use super::render_startup_record;

    /// The startup record is the first line an operator parses, so its shape
    /// is a contract: one JSON object per line, each carrying a level.
    #[test]
    fn startup_record_is_valid_json_with_a_level_field() {
        let rendered = render_startup_record();
        assert!(
            !rendered.trim().is_empty(),
            "the startup record must render at least one structured line"
        );
        for line in rendered.lines() {
            let record = serde_json::from_str::<serde_json::Value>(line)
                .expect("every rendered log line must be valid JSON");
            assert!(
                record.get("level").is_some(),
                "every rendered log line must carry a level field: {line}"
            );
        }
    }
}
