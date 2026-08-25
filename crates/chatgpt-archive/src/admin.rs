//! The admin router: liveness, readiness, metrics and version, on the
//! operator listener only.
//!
//! The admin plane carries NO error envelope: `/health/ready` returning 503
//! must tell an operator WHICH check failed, and these bodies are read by a
//! person and by a metrics scrape, not by a Ratatoskr client. All four routes
//! answer `Cache-Control: no-store`: a cached `ready` is a routing decision
//! made from stale data.

use std::sync::Arc;

use crate::telemetry::{GIT_SHA, ROLE, RUST_VERSION, SERVICE_NAME, VERSION};
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
/// The Prometheus text exposition format the `metrics` crate renders.
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4";

/// The future one readiness check resolves to.
type CheckFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>;

/// The stored shape of one registered check.
type CheckFn = Arc<dyn Fn() -> CheckFuture + Send + Sync>;

/// One registered readiness check.
struct CheckEntry {
    name: String,
    run: CheckFn,
}

impl CheckEntry {
    /// A cheap clone of the handle: the name is small and the closure is
    /// shared through `Arc`.
    fn clone_handle(&self) -> Self {
        Self {
            name: self.name.clone(),
            run: Arc::clone(&self.run),
        }
    }
}

/// The readiness facts a served process owns.
#[derive(Default)]
pub struct RuntimeState {
    checks: std::sync::Mutex<Vec<CheckEntry>>,
}

impl core::fmt::Debug for RuntimeState {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RuntimeState")
            .finish_non_exhaustive()
    }
}

/// One named readiness check result. Name-sorted in the response body, so two
/// consecutive bodies are byte-identical when nothing changes.
#[derive(Debug, serde::Serialize)]
pub struct CheckReport {
    /// The check's stable name.
    pub name: String,
    /// `ok` | `failed`.
    pub state: &'static str,
}

impl RuntimeState {
    /// Creates a runtime state with no checks registered yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one async readiness check under a stable name. Names should
    /// be bounded-cardinality facts such as `database`, never request data.
    pub fn register_check<F, Fut>(&self, name: &str, check: F)
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), String>> + Send + 'static,
    {
        let run = move || {
            let fut = check();
            Box::pin(fut) as CheckFuture
        };
        // A poisoned lock means a check panicked mid-report; refusing to
        // register would hide a dependency behind a mutex, so the entry is
        // simply dropped from a dead registry rather than unwrapped.
        if let Ok(mut entries) = self.checks.lock() {
            entries.push(CheckEntry {
                name: name.to_owned(),
                run: Arc::new(run),
            });
        }
    }

    /// Runs every registered check in name order.
    async fn run_checks(&self) -> Vec<CheckReport> {
        let entries = match self.checks.lock() {
            Ok(guard) => guard
                .iter()
                .map(CheckEntry::clone_handle)
                .collect::<Vec<_>>(),
            Err(poisoned) => poisoned
                .into_inner()
                .iter()
                .map(CheckEntry::clone_handle)
                .collect::<Vec<_>>(),
        };
        let mut reports = Vec::with_capacity(entries.len());
        for entry in entries {
            let outcome = (entry.run)().await;
            reports.push(CheckReport {
                name: entry.name,
                state: if outcome.is_ok() { "ok" } else { "failed" },
            });
        }
        reports.sort_by(|left, right| left.name.cmp(&right.name));
        reports
    }
}

/// What the four admin handlers read.
#[derive(Clone)]
struct AdminState {
    runtime: Arc<RuntimeState>,
    render_metrics: Arc<dyn Fn() -> String + Send + Sync>,
}

/// Builds the operator router: liveness, readiness, metrics, version.
pub fn admin_router<R>(state: Arc<RuntimeState>, render_metrics: R) -> Router
where
    R: Fn() -> String + Send + Sync + 'static,
{
    let state = AdminState {
        runtime: state,
        render_metrics: Arc::new(render_metrics),
    };
    Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
        .route("/metrics", get(metrics))
        .route("/version", get(version))
        .with_state(Arc::new(state))
        .layer(axum::middleware::map_response(no_store))
}

/// This process's async runtime is scheduling tasks and the HTTP server can
/// answer. It consults nothing external, ever.
async fn live() -> Json<Liveness> {
    Json(Liveness {
        state: "live",
        role: ROLE,
    })
}

/// Route new work to me: every registered check must pass.
async fn ready(State(state): State<Arc<AdminState>>) -> (StatusCode, Json<Readiness>) {
    let checks = state.runtime.run_checks().await;
    let ready = checks.iter().all(|check| check.state == "ok");
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(Readiness {
            state: if ready { "ready" } else { "not_ready" },
            role: ROLE,
            checks,
        }),
    )
}

/// Prometheus pull. One route calling the renderer: no second HTTP server, no
/// push gateway.
async fn metrics(State(state): State<Arc<AdminState>>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)],
        (state.render_metrics)(),
    )
}

/// The build identity, kept on the admin plane so a fingerprint is not public.
async fn version() -> Json<Version> {
    Json(Version {
        service: SERVICE_NAME,
        role: ROLE,
        version: VERSION,
        git_sha: GIT_SHA,
        rust_version: RUST_VERSION,
    })
}

/// `Cache-Control: no-store` on every admin response, including the bare 404.
async fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// `GET /health/live`.
#[derive(serde::Serialize)]
struct Liveness {
    /// Always `live`. The property is `state`, not `status`: `status` is a
    /// banned property name.
    state: &'static str,
    /// The one deployable role of this binary.
    role: &'static str,
}

/// `GET /health/ready`.
#[derive(serde::Serialize)]
struct Readiness {
    /// `ready` | `not_ready`.
    state: &'static str,
    /// The one deployable role of this binary.
    role: &'static str,
    /// Name-sorted, never a map, so two consecutive bodies are byte-identical.
    checks: Vec<CheckReport>,
}

/// `GET /version`.
#[allow(
    clippy::struct_field_names,
    reason = "the member names are the operator-facing JSON shape, not a naming choice"
)]
#[derive(serde::Serialize)]
struct Version {
    /// The one wire identity of this bounded context.
    service: &'static str,
    /// The one deployable role of this binary.
    role: &'static str,
    /// The crate version.
    version: &'static str,
    /// The build's git commit, or `unknown` outside a container build.
    git_sha: &'static str,
    /// The declared toolchain.
    rust_version: &'static str,
}
