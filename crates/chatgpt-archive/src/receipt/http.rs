//! The public receipt surface: `POST /exports` beside the admin plane.
//!
//! The handler authenticates first, refuses malformed declarations, streams
//! the body into the [`ArchiveReceiver`], and renders every failure through
//! the single error-envelope site. No archive byte, filename, or digest
//! fragment ever reaches a log line.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use axum::{Json, extract::State};
use futures_util::TryStreamExt as _;

use super::auth::TenantAuthenticator;
use super::{AcquisitionMode, ArchiveReceiver, ReceiptError, ReceiptOutcome};
use crate::error::{ArchiveError, FailureKind, Subsystem};

/// The acquisition-mode header the client must declare.
pub const HEADER_ACQUISITION: &str = "x-ratatoskr-acquisition";

/// What the receipt answers with on success.
#[derive(Debug, serde::Serialize)]
pub struct ReceiptAnswer {
    /// `stored` or `duplicate`.
    pub outcome: &'static str,
    /// The export identity: fresh for stored, pre-existing for duplicate.
    pub export_id: String,
    /// Lowercase SHA-256 hex of the received bytes.
    pub sha256_hex: String,
    /// Total bytes received.
    pub byte_length: u64,
}

/// The shared state of the public receipt routes.
#[derive(core::fmt::Debug)]
pub struct ReceiptApiState {
    receiver: ArchiveReceiver,
    authenticator: Arc<dyn TenantAuthenticator>,
}

impl ReceiptApiState {
    /// Binds a receiver and an authenticator into served state.
    #[must_use]
    pub fn new(receiver: ArchiveReceiver, authenticator: Arc<dyn TenantAuthenticator>) -> Self {
        Self {
            receiver,
            authenticator,
        }
    }
}

/// Builds the public receipt router: `POST /exports`, nothing else.
pub fn router(state: Arc<ReceiptApiState>) -> Router {
    Router::new()
        .route("/exports", post(create_export))
        .with_state(state)
}

/// The one place a receipt failure becomes an HTTP response.
fn respond(error: ReceiptError) -> Response {
    // Classify by consuming the error: internal arms log their diagnostics
    // exactly once and render the static 500; classified arms render their
    // static fault.
    let kind = match error {
        ReceiptError::DeclaredSizeExceeded | ReceiptError::StreamOvergrown => {
            FailureKind::PayloadTooLarge
        }
        ReceiptError::InvalidMediaType => FailureKind::InvalidRequest,
        ReceiptError::Repository(inner) => return internal_envelope(inner),
        ReceiptError::Storage(io) => return internal_envelope(io),
        other @ (ReceiptError::StreamFailed(_) | ReceiptError::StagingEvidenceLost) => {
            return internal_envelope(other);
        }
    };
    let boundary = ArchiveError::Rejected(kind);
    boundary.log();
    crate::fault::render_error_envelope(&boundary)
}

/// Logs one internal receipt failure at the boundary and renders the static
/// internal envelope.
fn internal_envelope<S>(source: S) -> Response
where
    S: std::error::Error + Send + Sync + 'static,
{
    let boundary = ArchiveError::internal(Subsystem::Receipt, source);
    boundary.log();
    crate::fault::render_error_envelope(&boundary)
}

async fn create_export(
    State(state): State<Arc<ReceiptApiState>>,
    headers: axum::http::HeaderMap,
    body: Body,
) -> Response {
    let credential = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    let Ok(principal) = state.authenticator.authenticate(credential) else {
        // One refusal for every authentication failure reason.
        let boundary = ArchiveError::Rejected(FailureKind::Unauthenticated);
        boundary.log();
        return crate::fault::render_error_envelope(&boundary);
    };

    let Some(mode_value) = headers
        .get(HEADER_ACQUISITION)
        .and_then(|v| v.to_str().ok())
    else {
        return crate::fault::reject(FailureKind::InvalidRequest);
    };
    let Some(mode) = AcquisitionMode::parse(mode_value) else {
        return crate::fault::reject(FailureKind::InvalidRequest);
    };

    let Some(media_type) = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    else {
        return crate::fault::reject(FailureKind::InvalidRequest);
    };

    let declared_length = match headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
    {
        Some(raw) => match raw.parse::<u64>() {
            Ok(length) => Some(length),
            Err(_) => return crate::fault::reject(FailureKind::InvalidRequest),
        },
        None => None,
    };

    let stream = http_body_util::BodyStream::new(body).map_ok(|frame| match frame.into_data() {
        Ok(data) => data,
        // Request uploads carry no trailers; an empty chunk keeps the
        // stream shape without inventing content.
        Err(_) => bytes::Bytes::new(),
    });

    match state
        .receiver
        .receive(&principal, mode, media_type, declared_length, stream)
        .await
    {
        Ok(outcome) => answer(outcome),
        Err(error) => respond(error),
    }
}

fn answer(outcome: ReceiptOutcome) -> Response {
    match outcome {
        ReceiptOutcome::Stored {
            export_id,
            sha256_hex,
            byte_length,
        } => (
            StatusCode::CREATED,
            Json(ReceiptAnswer {
                outcome: "stored",
                export_id: export_id.to_string(),
                sha256_hex,
                byte_length,
            }),
        )
            .into_response(),
        ReceiptOutcome::Duplicate {
            existing_export_id,
            sha256_hex,
            byte_length,
        } => (
            StatusCode::OK,
            Json(ReceiptAnswer {
                outcome: "duplicate",
                export_id: existing_export_id.to_string(),
                sha256_hex,
                byte_length,
            }),
        )
            .into_response(),
    }
}
