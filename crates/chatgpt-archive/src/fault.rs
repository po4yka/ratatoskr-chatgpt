//! THE error-envelope construction site, and the catch-panic responder.
//!
//! Every client-visible failure funnels through [`render_error_envelope`];
//! `reject` is a convenience that renders a classified kind through it. The
//! panic responder's body is static text and never carries the payload.

use std::any::Any;

use axum::Json;
use axum::body::Body;
use axum::response::{IntoResponse as _, Response};
use http::StatusCode;
use ratatoskr_error_contracts::ErrorEnvelope;
use tower_http::catch_panic::CatchPanicLayer;

use crate::error::{ArchiveError, FailureKind};

/// The responder signature the `tower-http` catch-panic contract hands over.
type PanicResponder = fn(Box<dyn Any + Send + 'static>) -> http::Response<Body>;

/// THE place an [`ErrorEnvelope`] is constructed; there is no other
/// construction call in this repository.
///
/// The `Internal` arm of `error` is unreachable from here in terms of content:
/// this function reads [`ArchiveError::fault`] and nothing else, so no
/// `subsystem` and no `source` has a path into a response body. Callers log
/// the error with [`ArchiveError::log`] before rendering.
#[must_use = "a rendered envelope that is dropped silently loses the failure"]
pub fn render_error_envelope(error: &ArchiveError) -> Response {
    let fault = error.fault();
    let envelope = ErrorEnvelope::new(fault.code.clone(), fault.message.clone(), fault.retryable);
    (fault.status, Json(envelope)).into_response()
}

/// Refuse a request with a named failure, rendered through the single
/// construction site.
#[must_use]
pub fn reject(kind: FailureKind) -> Response {
    render_error_envelope(&ArchiveError::Rejected(kind))
}

/// The layer every served router applies: a panicking handler becomes a
/// plain-text 500 instead of a dropped connection or a leaked payload.
#[must_use]
pub fn catch_panics() -> CatchPanicLayer<PanicResponder> {
    CatchPanicLayer::custom(panic_responder)
}

/// The catch-panic responder: extracts the payload for the log record only,
/// and answers with a static text body.
// The payload arrives by value because the `tower-http` `ResponseForPanic`
// contract hands it over as an owned `Box`; only its contents are read here.
#[allow(
    clippy::needless_pass_by_value,
    reason = "the tower-http responder contract requires taking ownership of the payload box"
)]
fn panic_responder(payload: Box<dyn Any + Send + 'static>) -> http::Response<Body> {
    let mut response = http::Response::new(Body::from(INTERNAL_PANIC_TEXT));
    *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    response
        .extensions_mut()
        .insert(CaughtPanic(describe(payload.as_ref())));
    response
}

/// The static text a panicking handler produces; no payload ever joins it.
const INTERNAL_PANIC_TEXT: &str = "An internal failure prevented the request from completing.";

/// The failure a caught panic represents, carried to the one logging site.
///
/// Cloneable because `http::Extensions` requires it, and an error because it
/// becomes the source of an [`ArchiveError::Internal`].
#[derive(Debug, Clone)]
pub struct CaughtPanic(String);

impl core::fmt::Display for CaughtPanic {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "a request handler panicked: {}", self.0)
    }
}

impl std::error::Error for CaughtPanic {}

/// The panic payload as text, for the log record only.
fn describe(payload: &(dyn Any + Send + 'static)) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        return (*message).to_owned();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "the payload is not a string".to_owned()
}
