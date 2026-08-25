//! The archive error taxonomy and its closed public projection.
//!
//! The two-arm split is the security property: the client-visible arm has a
//! unit payload, so no caller-influenced or dependency-authored text has
//! anywhere to sit, and the diagnostics arm carries data the envelope
//! renderer cannot read.

use std::sync::LazyLock;

use http::StatusCode;
use ratatoskr_error_contracts::ErrorCode;
use ratatoskr_identifiers::SafeMessage;

/// Everything that can fail inside the process and reach an HTTP boundary.
///
/// [`ArchiveError::Rejected`] has a unit payload, so there is nowhere for
/// caller-influenced or dependency-authored text to sit; a caller who wants to
/// smuggle a provider message into a 4xx has to change this enum, which is a
/// reviewed diff. [`ArchiveError::Internal`] carries diagnostics the envelope
/// renderer cannot read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ArchiveError {
    /// A failure the caller may see and act on. The public code, status,
    /// retryability and message all come from the kind's static table.
    #[error("{0}")]
    Rejected(FailureKind),

    /// A failure inside the service. `source` is logged exactly once, at the
    /// boundary, and never serialized.
    #[error("internal failure in {subsystem}")]
    Internal {
        /// Which part of the process failed. A telemetry attribute, never a
        /// client-visible fact.
        subsystem: Subsystem,
        /// The diagnostics. Logged once at the boundary; never rendered.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
}

impl ArchiveError {
    /// Constructs an internal failure from any error.
    pub fn internal(
        subsystem: Subsystem,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Internal {
            subsystem,
            source: Box::new(source),
        }
    }

    /// The public projection. Exhaustive; a new variant does not compile
    /// until it has one.
    #[must_use]
    pub fn fault(&self) -> &'static PublicFault {
        match self {
            Self::Rejected(kind) => kind.fault(),
            Self::Internal { .. } => &INTERNAL,
        }
    }

    /// Writes the diagnostics exactly once, at the boundary, and nowhere else.
    pub fn log(&self) {
        let fault = self.fault();
        let code = fault.code.as_str();
        let status = fault.status.as_u16();
        match self {
            Self::Internal { subsystem, source } => {
                tracing::error!(
                    subsystem = %subsystem,
                    code,
                    status,
                    chain = %source_chain(source.as_ref()),
                    "internal failure"
                );
            }
            Self::Rejected(kind) if kind.fault().status.is_server_error() => {
                tracing::warn!(kind = %kind, code, status, "request rejected");
            }
            Self::Rejected(kind) => {
                tracing::info!(kind = %kind, code, status, "request rejected");
            }
        }
    }
}

/// The whole source chain of an error, innermost cause last.
fn source_chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut chain = error.to_string();
    let mut current = error.source();
    while let Some(cause) = current {
        chain.push_str(": ");
        chain.push_str(&cause.to_string());
        current = cause.source();
    }
    chain
}

/// Which part of the process failed. Bounded-cardinality telemetry only:
/// never on a wire, never in a response body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Subsystem {
    /// Reading or validating the typed configuration.
    Config,
    /// The subscriber, the exporter or an instrument.
    Telemetry,
    /// The HTTP harness: a listener, a middleware, or a handler.
    Http,
    /// The database pool, the schema, or a query.
    Persistence,
}

impl core::fmt::Display for Subsystem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Config => "config",
            Self::Telemetry => "telemetry",
            Self::Http => "http",
            Self::Persistence => "persistence",
        })
    }
}

/// The closed public failure taxonomy.
///
/// Each variant fixes a code, a status, a retry class and a message. Nothing
/// is derived from data, and there is deliberately NO `Internal` variant: an
/// internal failure is unreachable through [`ArchiveError::Rejected`], so it
/// is structurally impossible to attach a caller-influenced message or a
/// non-500 status to one.
///
/// The routing kinds have no producer yet: the first public routes arrive with
/// implementation plan item 2, and the rendering boundary they will use is
/// proven by the contract tests now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FailureKind {
    /// No route matched. Produced by the future public router's fallback.
    RouteNotFound,
    /// The route exists, the method does not.
    MethodNotAllowed,
    /// No credential, or one that does not authenticate. One variant on
    /// purpose: missing, malformed, revoked and expired credentials are
    /// indistinguishable to a caller.
    Unauthenticated,
    /// The resource does not exist, or exists and is not this principal's.
    NotFound,
    /// The request body is not what the route accepts.
    InvalidRequest,
}

impl FailureKind {
    /// Every kind, in status order. The array length is the documented count,
    /// so adding a variant without updating it does not compile.
    pub const ALL: [Self; 5] = [
        Self::InvalidRequest,
        Self::Unauthenticated,
        Self::NotFound,
        Self::RouteNotFound,
        Self::MethodNotAllowed,
    ];

    /// The only thing a client ever learns about this failure.
    #[must_use]
    pub fn fault(self) -> &'static PublicFault {
        match self {
            Self::RouteNotFound => &ROUTE_NOT_FOUND,
            Self::MethodNotAllowed => &METHOD_NOT_ALLOWED,
            Self::Unauthenticated => &UNAUTHENTICATED,
            Self::NotFound => &NOT_FOUND,
            Self::InvalidRequest => &INVALID_REQUEST,
        }
    }
}

impl core::fmt::Display for FailureKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.fault().code.as_str())
    }
}

/// The only thing a client ever learns about a failure.
///
/// Every member is owned static data produced once by a [`LazyLock`]. There is
/// no `String`, no `format!`, and no borrow of the error anywhere in this
/// type, so no runtime value has a path into a public code, message or status.
#[derive(Debug)]
pub struct PublicFault {
    /// The HTTP status the failure renders as.
    pub status: StatusCode,
    /// The stable, machine-actionable code — the only member a consumer may
    /// branch on.
    pub code: ErrorCode,
    /// The human-readable explanation. Never machine-parsed, never stable
    /// across releases.
    pub message: SafeMessage,
    /// Whether repeating the identical request may succeed later without
    /// operator action. Explicit, never inferred from the code by a consumer.
    pub retryable: bool,
}

/// Builds one table entry from compile-time contract constants.
#[allow(
    clippy::expect_used,
    reason = "FailureKind's code and message strings are compile-time constants, proved \
              parseable by the boundary tests; a build whose table is malformed is broken \
              before it serves a request"
)]
fn entry(status: StatusCode, code: &str, message: &str, retryable: bool) -> PublicFault {
    PublicFault {
        status,
        code: ErrorCode::parse(code).expect("a fault code must satisfy the contract grammar"),
        message: SafeMessage::parse(message).expect("a fault message must be a safe message"),
        retryable,
    }
}

/// `chatgpt.request.invalid` — 400. Not retryable: the same bytes fail again.
static INVALID_REQUEST: LazyLock<PublicFault> = LazyLock::new(|| {
    entry(
        StatusCode::BAD_REQUEST,
        "chatgpt.request.invalid",
        "The request body is not what this endpoint accepts.",
        false,
    )
});

/// `chatgpt.auth.unauthenticated` — 401. Deliberately uninformative: the
/// difference between a missing and an expired credential is an oracle.
static UNAUTHENTICATED: LazyLock<PublicFault> = LazyLock::new(|| {
    entry(
        StatusCode::UNAUTHORIZED,
        "chatgpt.auth.unauthenticated",
        "Authentication is required.",
        false,
    )
});

/// `chatgpt.archive.not_found` — 404. One answer for "not there" and "not
/// yours": authorization runs before existence is disclosed.
static NOT_FOUND: LazyLock<PublicFault> = LazyLock::new(|| {
    entry(
        StatusCode::NOT_FOUND,
        "chatgpt.archive.not_found",
        "The requested resource does not exist.",
        false,
    )
});

/// `chatgpt.route.not_found` — 404 for an unknown path.
static ROUTE_NOT_FOUND: LazyLock<PublicFault> = LazyLock::new(|| {
    entry(
        StatusCode::NOT_FOUND,
        "chatgpt.route.not_found",
        "The requested endpoint does not exist.",
        false,
    )
});

/// `chatgpt.request.method_not_allowed` — 405.
static METHOD_NOT_ALLOWED: LazyLock<PublicFault> = LazyLock::new(|| {
    entry(
        StatusCode::METHOD_NOT_ALLOWED,
        "chatgpt.request.method_not_allowed",
        "This endpoint does not accept that method.",
        false,
    )
});

/// The projection of every [`ArchiveError::Internal`]: one static shape, so a
/// dependency's own error text has no path to a client.
static INTERNAL: LazyLock<PublicFault> = LazyLock::new(|| {
    entry(
        StatusCode::INTERNAL_SERVER_ERROR,
        "chatgpt.server.internal",
        "An internal failure prevented the request from completing.",
        false,
    )
});
