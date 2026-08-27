//! The public receipt surface: `POST /exports` beside the admin plane.
//!
//! The handler authenticates first, refuses malformed declarations, streams
//! the body into the [`ArchiveReceiver`], and renders every failure through
//! the single error-envelope site. No archive byte, filename, or digest
//! fragment ever reaches a log line.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use axum::{Json, extract::State};
use futures_util::TryStreamExt as _;

use super::auth::{TenantAuthenticator, TenantPrincipal};
use super::{AcquisitionMode, ArchiveReceiver, PlatformOperation, ReceiptError, ReceiptOutcome};
use crate::error::{ArchiveError, FailureKind, Subsystem};

/// The acquisition-mode header the client must declare.
pub const HEADER_ACQUISITION: &str = "x-ratatoskr-acquisition";
const HEADER_PLATFORM_USER_ID: &str = "x-ratatoskr-user-id";
const HEADER_PLATFORM_DEVICE_ID: &str = "x-ratatoskr-device-id";
const HEADER_CORRELATION_ID: &str = "x-correlation-id";
const HEADER_OPERATION_ID: &str = "x-ratatoskr-operation-id";
const HEADER_ARCHIVE_SHA256: &str = "x-ratatoskr-archive-sha256";
const HEADER_ARCHIVE_BYTE_SIZE: &str = "x-ratatoskr-archive-byte-size";

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
    platform_accounts: HashMap<uuid::Uuid, TenantPrincipal>,
}

impl ReceiptApiState {
    /// Binds a receiver and an authenticator into served state.
    #[must_use]
    pub fn new(receiver: ArchiveReceiver, authenticator: Arc<dyn TenantAuthenticator>) -> Self {
        Self {
            receiver,
            authenticator,
            platform_accounts: HashMap::new(),
        }
    }

    /// Binds receipt state with the configured Platform-to-account mappings.
    #[must_use]
    pub fn new_with_platform_accounts<I>(
        receiver: ArchiveReceiver,
        authenticator: Arc<dyn TenantAuthenticator>,
        accounts: I,
    ) -> Self
    where
        I: IntoIterator<Item = (&'static str, &'static str)>,
    {
        let platform_accounts = accounts
            .into_iter()
            .filter_map(|(user, account)| {
                user.parse::<uuid::Uuid>().ok().map(|user_id| {
                    (
                        user_id,
                        TenantPrincipal {
                            account_external_ref: account.to_owned(),
                        },
                    )
                })
            })
            .collect();
        Self {
            receiver,
            authenticator,
            platform_accounts,
        }
    }

    /// Binds receipt state from validated Platform-user account mappings.
    #[must_use]
    pub fn new_with_platform_account_ids(
        receiver: ArchiveReceiver,
        authenticator: Arc<dyn TenantAuthenticator>,
        accounts: &[(uuid::Uuid, String)],
    ) -> Self {
        let platform_accounts = accounts
            .iter()
            .map(|(user_id, account)| {
                (
                    *user_id,
                    TenantPrincipal {
                        account_external_ref: account.clone(),
                    },
                )
            })
            .collect();
        Self {
            receiver,
            authenticator,
            platform_accounts,
        }
    }
}

/// Builds the public receipt router: `POST /exports`, nothing else.
pub fn router(state: Arc<ReceiptApiState>) -> Router {
    Router::new()
        .route("/exports", post(create_export))
        .route("/v1/ai-archives/receipt", post(receive_platform_archive))
        .with_state(state)
}

/// The trusted, loopback-only receipt Platform calls after device authentication.
async fn receive_platform_archive(
    State(state): State<Arc<ReceiptApiState>>,
    headers: axum::http::HeaderMap,
    body: Body,
) -> Response {
    if headers.contains_key(axum::http::header::AUTHORIZATION) {
        return crate::fault::reject(FailureKind::Unauthenticated);
    }
    let Some((principal, expected_sha256, expected_size, operation)) =
        platform_claims(&state, &headers)
    else {
        return crate::fault::reject(FailureKind::Unauthenticated);
    };
    let Some(media_type) = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    else {
        return crate::fault::reject(FailureKind::InvalidRequest);
    };
    let stream = http_body_util::BodyStream::new(body).map_ok(|frame| match frame.into_data() {
        Ok(data) => data,
        Err(_) => bytes::Bytes::new(),
    });
    match state
        .receiver
        .receive_platform_archive(
            &principal,
            media_type,
            expected_size,
            &expected_sha256,
            operation,
            stream,
        )
        .await
    {
        Ok(ReceiptOutcome::Stored { .. } | ReceiptOutcome::Duplicate { .. }) => {
            StatusCode::ACCEPTED.into_response()
        }
        Err(error) => respond(error),
    }
}

fn platform_claims(
    state: &ReceiptApiState,
    headers: &axum::http::HeaderMap,
) -> Option<(TenantPrincipal, String, u64, PlatformOperation)> {
    let parse = |name: &'static str| headers.get(name)?.to_str().ok();
    let user_id = parse(HEADER_PLATFORM_USER_ID)?.parse::<uuid::Uuid>().ok()?;
    let _device_id = parse(HEADER_PLATFORM_DEVICE_ID)?
        .parse::<uuid::Uuid>()
        .ok()?;
    let correlation = parse(HEADER_CORRELATION_ID)?;
    let operation_id = parse(HEADER_OPERATION_ID)?.parse::<uuid::Uuid>().ok()?;
    let sha256 = parse(HEADER_ARCHIVE_SHA256)?;
    let byte_size = parse(HEADER_ARCHIVE_BYTE_SIZE)?.parse::<u64>().ok()?;
    if correlation.is_empty()
        || correlation.len() > 200
        || byte_size == 0
        || sha256.len() != 64
        || !sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    state
        .platform_accounts
        .get(&user_id)
        .cloned()
        .map(|principal| {
            (
                principal,
                sha256.to_owned(),
                byte_size,
                PlatformOperation { operation_id },
            )
        })
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
        ReceiptError::InvalidMediaType | ReceiptError::DigestMismatch => {
            FailureKind::InvalidRequest
        }
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
            ..
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
