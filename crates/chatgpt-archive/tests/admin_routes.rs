//! Contract tests for the operator admin plane.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use ratatoskr_chatgpt_archive::admin::{RuntimeState, admin_router};
use tower::ServiceExt as _;

/// Builds a served admin router with no readiness checks registered.
fn app() -> Router {
    let state = Arc::new(RuntimeState::new());
    admin_router(state, || "chatgpt_build_info 1\n".to_owned())
}

async fn body_string(
    response: axum::response::Response,
) -> Result<String, Box<dyn std::error::Error>> {
    let bytes = response.into_body().collect().await?.to_bytes();
    Ok(String::from_utf8(bytes.to_vec())?)
}

/// Liveness consults nothing: it answers 200 `live` even when nothing is.
#[tokio::test]
async fn live_answers_without_database() -> Result<(), Box<dyn std::error::Error>> {
    let response = app()
        .oneshot(
            Request::get("/health/live")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("in-process request must not fail");

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await?;
    let json =
        serde_json::from_str::<serde_json::Value>(&body).expect("liveness body must be JSON");
    assert_eq!(json.get("state").and_then(|s| s.as_str()), Some("live"));
    Ok(())
}

/// A failed readiness check flips the status and names the check.
#[tokio::test]
async fn ready_lists_failing_check_by_name() -> Result<(), Box<dyn std::error::Error>> {
    async fn failing() -> Result<(), String> {
        Err("database refused".to_owned())
    }

    let state = Arc::new(RuntimeState::new());
    state.register_check("database", failing);
    let app = admin_router(state, String::new);

    let response = app
        .oneshot(
            Request::get("/health/ready")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("in-process request must not fail");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = body_string(response).await?;
    let json = serde_json::from_str::<serde_json::Value>(&body)?;
    assert_eq!(
        json.get("state").and_then(|s| s.as_str()),
        Some("not_ready")
    );
    let checks = json
        .get("checks")
        .and_then(|checks| checks.as_array())
        .expect("readiness must list checks");
    assert!(
        checks
            .iter()
            .any(|check| check.get("name").and_then(|n| n.as_str()) == Some("database"))
    );
    Ok(())
}

/// Version identifies the build.
#[tokio::test]
async fn version_reports_build_identity() -> Result<(), Box<dyn std::error::Error>> {
    let response = app()
        .oneshot(
            Request::get("/version")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("in-process request must not fail");

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await?;
    let json = serde_json::from_str::<serde_json::Value>(&body)?;
    assert_eq!(
        json.get("service").and_then(|s| s.as_str()),
        Some(ratatoskr_chatgpt_archive::telemetry::SERVICE_NAME)
    );
    assert_eq!(
        json.get("version").and_then(|v| v.as_str()),
        Some(env!("CARGO_PKG_VERSION"))
    );
    assert!(json.get("git_sha").is_some());
    Ok(())
}

/// Every admin response refuses caching, including metrics and version.
#[tokio::test]
async fn admin_responses_carry_no_store() -> Result<(), Box<dyn std::error::Error>> {
    for path in ["/health/live", "/health/ready", "/metrics", "/version"] {
        let response = app()
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).expect("request"))
            .await
            .expect("request to an admin route must not fail");

        let header = response
            .headers()
            .get("cache-control")
            .expect("every admin response must carry Cache-Control");
        assert_eq!(header, "no-store", "{path} must refuse caching");
    }
    Ok(())
}
