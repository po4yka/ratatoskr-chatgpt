//! Contract tests for the error taxonomy and its single rendering site.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use http_body_util::BodyExt as _;
use ratatoskr_chatgpt_archive::error::{ArchiveError, FailureKind, Subsystem};
use ratatoskr_chatgpt_archive::fault;
use tower::ServiceExt as _;

/// Reads a response body to bytes for assertions.
async fn body_bytes(
    response: axum::response::Response,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let bytes = response.into_body().collect().await?.to_bytes();
    Ok(bytes.to_vec())
}

/// A classified kind renders its static status, code, message, retry class.
#[tokio::test]
async fn rejected_kind_maps_to_envelope() -> Result<(), Box<dyn std::error::Error>> {
    let response = fault::reject(FailureKind::RouteNotFound);

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = String::from_utf8(body_bytes(response).await?)?;
    let envelope = serde_json::from_str::<serde_json::Value>(&body)?;
    assert_eq!(
        envelope.get("code").and_then(serde_json::Value::as_str),
        Some("chatgpt.route.not_found")
    );
    assert_eq!(
        envelope
            .get("retryable")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    let message = envelope
        .get("message")
        .and_then(|message| message.as_str())
        .unwrap_or_default();
    assert!(!message.is_empty());
    Ok(())
}

/// An internal failure's source chain never reaches the rendered body.
#[tokio::test]
async fn internal_error_leaks_no_source_detail() -> Result<(), Box<dyn std::error::Error>> {
    let error = ArchiveError::internal(
        Subsystem::Persistence,
        std::io::Error::other(
            "credentials rejected for role chatgpt_secret_marker on the archive database",
        ),
    );

    let response = fault::render_error_envelope(&error);
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let body = String::from_utf8(body_bytes(response).await?)?;
    assert!(
        !body.contains("chatgpt_secret_marker"),
        "the source chain leaked into the envelope body: {body}"
    );
    assert!(
        !body.contains("credentials rejected"),
        "the source chain leaked into the envelope body: {body}"
    );
    Ok(())
}

/// A panicking handler answers with static text and the router keeps serving.
#[tokio::test]
async fn panic_renders_static_500() -> Result<(), Box<dyn std::error::Error>> {
    // The panicking handler is the fixture: the panic itself is the behavior
    // under test, so the production-code ban is lifted for this one site.
    #[allow(
        clippy::panic,
        reason = "the handler must actually panic; that is the contract being tested"
    )]
    async fn boom() -> axum::response::Response {
        panic!("kaboom secret-detail");
    }
    async fn alive() -> &'static str {
        "alive"
    }

    let app = Router::new()
        .route("/boom", get(boom))
        .route("/alive", get(alive))
        .layer(fault::catch_panics());

    let boom = app
        .clone()
        .oneshot(Request::get("/boom").body(Body::empty()).expect("request"))
        .await
        .expect("a caught panic must still produce a response");
    assert_eq!(boom.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let text = String::from_utf8(body_bytes(boom).await?)?;
    assert!(
        !text.contains("kaboom") && !text.contains("secret-detail"),
        "the panic payload leaked into the response: {text}"
    );

    let after = app
        .oneshot(Request::get("/alive").body(Body::empty()).expect("request"))
        .await
        .expect("the router must keep serving");
    assert_eq!(after.status(), StatusCode::OK);
    Ok(())
}
