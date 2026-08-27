//! Contract tests for the public receipt surface: `POST /exports`.
//!
//! The router is exercised through `tower::ServiceExt::oneshot` against the
//! real handler, the fake repository, and temp directories — no network, no
//! database.

// Test bodies fail through `panic!`/`expect`; assertions are the contract,
// and the fixture helpers sit outside `#[test]` functions.
#![allow(clippy::expect_used, reason = "test failures report through panics")]
#![allow(clippy::panic, reason = "test failures report through panics")]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use ratatoskr_chatgpt_archive::receipt::auth::ConfigTenantAuthenticator;
use ratatoskr_chatgpt_archive::receipt::http::{HEADER_ACQUISITION, ReceiptApiState};
use ratatoskr_chatgpt_archive::test_support::FakeReceiptRepository;
use ratatoskr_chatgpt_archive::{ArchiveReceiver, BlobStore};
use tower::ServiceExt as _;

/// A one-shot byte stream over fixed chunks.
fn byte_stream(chunks: Vec<&'static [u8]>) -> Body {
    let iter = chunks.into_iter().map(Ok::<_, std::convert::Infallible>);
    Body::from_stream(futures_util::stream::iter(iter))
}

struct ApiFixture {
    app: axum::Router,
    repository: Arc<FakeReceiptRepository>,
    #[allow(dead_code)]
    blob_root: tempfile::TempDir,
    #[allow(dead_code)]
    staging: tempfile::TempDir,
}

fn fixture(max_archive_bytes: u64) -> ApiFixture {
    let blob_root = tempfile::tempdir().expect("blob root");
    let staging = tempfile::tempdir().expect("staging root");
    let blob = BlobStore::new(blob_root.path()).expect("blob store");
    let repository = Arc::new(FakeReceiptRepository::new());
    let receiver = ArchiveReceiver::new(
        blob,
        Arc::clone(&repository) as Arc<dyn ratatoskr_chatgpt_archive::ReceiptRepository>,
        staging.path().to_path_buf(),
        max_archive_bytes,
    )
    .expect("receiver");
    let authenticator = Arc::new(ConfigTenantAuthenticator::new(vec![(
        "tok-alpha".to_owned(),
        "acc-one".to_owned(),
    )]));
    let app = ratatoskr_chatgpt_archive::receipt::http::router(Arc::new(
        ReceiptApiState::new_with_platform_accounts(
            receiver,
            authenticator,
            [("00000000-0000-0000-0000-000000000011", "acc-one")],
        ),
    ));
    ApiFixture {
        app,
        repository,
        blob_root,
        staging,
    }
}

fn request(
    method: &str,
    token: Option<&str>,
    acquisition: Option<&str>,
    media_type: Option<&str>,
    declared: Option<u64>,
    body: Body,
) -> Request<Body> {
    request_at(
        "/exports",
        method,
        token,
        acquisition,
        media_type,
        declared,
        body,
    )
}

fn request_at(
    uri: &str,
    method: &str,
    token: Option<&str>,
    acquisition: Option<&str>,
    media_type: Option<&str>,
    declared: Option<u64>,
    body: Body,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(axum::http::header::HOST, "127.0.0.1");
    if let Some(token) = token {
        builder = builder.header(axum::http::header::AUTHORIZATION, token);
    }
    if let Some(acquisition) = acquisition {
        builder = builder.header(HEADER_ACQUISITION, acquisition);
    }
    if let Some(media_type) = media_type {
        builder = builder.header(axum::http::header::CONTENT_TYPE, media_type);
    }
    if let Some(declared) = declared {
        builder = builder.header(axum::http::header::CONTENT_LENGTH, declared.to_string());
    }
    builder.body(body).expect("request builds")
}

async fn send(app: &axum::Router, request: Request<Body>) -> (StatusCode, serde_json::Value) {
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("infallible service");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json)
}

const GOOD_BODY: &[&[u8]] = &[b"an-archive-", b"payload"];

/// A healthy authenticated upload answers 201 carrying its new identity.
#[tokio::test]
async fn stored_receipt_answers_201_with_identity() {
    let fixture = fixture(u64::MAX);
    let total: u64 = GOOD_BODY.iter().map(|chunk| chunk.len() as u64).sum();
    let (status, json) = send(
        &fixture.app,
        request(
            "POST",
            Some("Bearer tok-alpha"),
            Some("consumer_export"),
            Some("application/zip"),
            Some(total),
            byte_stream(GOOD_BODY.to_vec()),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(json.get("outcome").and_then(|v| v.as_str()), Some("stored"));
    assert!(json.get("export_id").and_then(|v| v.as_str()).is_some());
    assert!(
        json.get("sha256_hex")
            .and_then(|v| v.as_str())
            .is_some_and(|hex| hex.len() == 64)
    );
    assert_eq!(
        json.get("byte_length").and_then(serde_json::Value::as_u64),
        Some(total)
    );
}

/// An identical re-upload answers 200 naming the existing export.
#[tokio::test]
async fn duplicate_receipt_answers_200_naming_the_existing_export() {
    let fixture = fixture(u64::MAX);
    let total: u64 = GOOD_BODY.iter().map(|chunk| chunk.len() as u64).sum();
    let first = send(
        &fixture.app,
        request(
            "POST",
            Some("Bearer tok-alpha"),
            Some("consumer_export"),
            Some("application/zip"),
            Some(total),
            byte_stream(GOOD_BODY.to_vec()),
        ),
    )
    .await;
    assert_eq!(first.0, StatusCode::CREATED);

    let (status, second) = send(
        &fixture.app,
        request(
            "POST",
            Some("Bearer tok-alpha"),
            Some("consumer_export"),
            Some("application/zip"),
            Some(total),
            byte_stream(GOOD_BODY.to_vec()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        second.get("outcome").and_then(|v| v.as_str()),
        Some("duplicate")
    );
    assert_eq!(
        second.get("export_id"),
        first.1.get("export_id"),
        "the duplicate names the pre-existing export"
    );
}

/// Missing and unknown credentials render the same unauthenticated envelope.
#[tokio::test]
async fn unauthenticated_requests_render_the_401_envelope() {
    let fixture = fixture(u64::MAX);
    for token in [None, Some("Bearer wrong-token"), Some("not-a-scheme")] {
        let (status, json) = send(
            &fixture.app,
            request(
                "POST",
                token,
                Some("consumer_export"),
                Some("application/zip"),
                Some(12),
                byte_stream(GOOD_BODY.to_vec()),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "token {token:?}");
        assert!(
            json.get("error").is_some() || json.get("code").is_some(),
            "the refusal renders the error envelope: {json}"
        );
        assert!(
            fixture.repository.exports_snapshot().is_empty(),
            "no state may exist behind a refused credential"
        );
    }
}

/// A declaration beyond the cap renders the payload-too-large envelope.
#[tokio::test]
async fn oversized_declaration_renders_the_413_envelope() {
    let fixture = fixture(1024);
    let (status, json) = send(
        &fixture.app,
        request(
            "POST",
            Some("Bearer tok-alpha"),
            Some("consumer_export"),
            Some("application/zip"),
            Some(2048),
            byte_stream(GOOD_BODY.to_vec()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        json.get("code").and_then(|v| v.as_str()),
        Some("chatgpt.request.too_large")
    );
}

/// Anything other than POST on the receipt path answers 405.
#[tokio::test]
async fn wrong_method_answers_405() {
    let fixture = fixture(u64::MAX);
    let (status, _) = send(
        &fixture.app,
        request(
            "GET",
            Some("Bearer tok-alpha"),
            None,
            None,
            None,
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}

/// Platform forwards a verified archive only to its private receipt route;
/// the normal tenant receipt route must not be used as a substitute.
#[tokio::test]
async fn platform_archive_receipt_route_is_available() {
    let fixture = fixture(u64::MAX);
    let total: u64 = GOOD_BODY.iter().map(|chunk| chunk.len() as u64).sum();
    let mut request = request_at(
        "/v1/ai-archives/receipt",
        "POST",
        None,
        None,
        Some("application/zip"),
        Some(total),
        byte_stream(GOOD_BODY.to_vec()),
    );
    request.headers_mut().insert(
        "x-ratatoskr-user-id",
        "00000000-0000-0000-0000-000000000011"
            .parse()
            .expect("user header"),
    );
    request.headers_mut().insert(
        "x-ratatoskr-device-id",
        "00000000-0000-0000-0000-000000000012"
            .parse()
            .expect("device header"),
    );
    request.headers_mut().insert(
        "x-correlation-id",
        "correlation:00000000-0000-0000-0000-000000000013"
            .parse()
            .expect("correlation header"),
    );
    request.headers_mut().insert(
        "x-ratatoskr-operation-id",
        "00000000-0000-0000-0000-000000000014"
            .parse()
            .expect("operation header"),
    );
    request.headers_mut().insert(
        "x-ratatoskr-archive-sha256",
        "1d99fb3667bdf788ccfcc99afe73c8ee02480e5aa3afecf3403c85eec47b84db"
            .parse()
            .expect("digest header"),
    );
    request.headers_mut().insert(
        "x-ratatoskr-archive-byte-size",
        total.to_string().parse().expect("size header"),
    );
    let (status, _) = send(&fixture.app, request).await;

    assert_eq!(status, StatusCode::ACCEPTED);
}

/// Raw storage is terminally truthful: until the parser has established
/// normalized coverage, Platform receives exactly one partial/unknown report.
#[tokio::test]
async fn platform_archive_receipt_records_one_unknown_partial_terminal_report() {
    let fixture = fixture(u64::MAX);
    let total: u64 = GOOD_BODY.iter().map(|chunk| chunk.len() as u64).sum();
    let mut request = request_at(
        "/v1/ai-archives/receipt",
        "POST",
        None,
        None,
        Some("application/zip"),
        Some(total),
        byte_stream(GOOD_BODY.to_vec()),
    );
    request.headers_mut().insert(
        "x-ratatoskr-user-id",
        "00000000-0000-0000-0000-000000000011"
            .parse()
            .expect("user header"),
    );
    request.headers_mut().insert(
        "x-ratatoskr-device-id",
        "00000000-0000-0000-0000-000000000012"
            .parse()
            .expect("device header"),
    );
    request.headers_mut().insert(
        "x-correlation-id",
        "correlation:00000000-0000-0000-0000-000000000013"
            .parse()
            .expect("correlation header"),
    );
    request.headers_mut().insert(
        "x-ratatoskr-operation-id",
        "00000000-0000-0000-0000-000000000014"
            .parse()
            .expect("operation header"),
    );
    request.headers_mut().insert(
        "x-ratatoskr-archive-sha256",
        "1d99fb3667bdf788ccfcc99afe73c8ee02480e5aa3afecf3403c85eec47b84db"
            .parse()
            .expect("digest header"),
    );
    request.headers_mut().insert(
        "x-ratatoskr-archive-byte-size",
        total.to_string().parse().expect("size header"),
    );

    let (status, _) = send(&fixture.app, request).await;

    assert_eq!(status, StatusCode::ACCEPTED);
    let reports = fixture.repository.operation_reports();
    assert_eq!(reports.len(), 1, "one receipt creates one terminal report");
    assert_eq!(
        reports[0]["operation_id"],
        "00000000-0000-0000-0000-000000000014"
    );
    assert_eq!(reports[0]["status"], "partially_succeeded");
    assert_eq!(
        reports[0]["results"][0]["ai_archive_import_summary"]["provider"],
        "chatgpt"
    );
    assert_eq!(
        reports[0]["results"][0]["ai_archive_import_summary"]["completeness"],
        "unknown"
    );
    assert_eq!(
        reports[0]["results"][0]["ai_archive_import_summary"]["gap_count"],
        1
    );
}

/// Edge's digest claim is archive identity, not advisory metadata: bytes that
/// do not match it must never become a stored export.
#[tokio::test]
async fn platform_archive_receipt_refuses_mismatched_digest_without_storing() {
    let fixture = fixture(u64::MAX);
    let total: u64 = GOOD_BODY.iter().map(|chunk| chunk.len() as u64).sum();
    let mut request = request_at(
        "/v1/ai-archives/receipt",
        "POST",
        None,
        None,
        Some("application/zip"),
        Some(total),
        byte_stream(GOOD_BODY.to_vec()),
    );
    request.headers_mut().insert(
        "x-ratatoskr-user-id",
        "00000000-0000-0000-0000-000000000011"
            .parse()
            .expect("user header"),
    );
    request.headers_mut().insert(
        "x-ratatoskr-device-id",
        "00000000-0000-0000-0000-000000000012"
            .parse()
            .expect("device header"),
    );
    request.headers_mut().insert(
        "x-correlation-id",
        "correlation:00000000-0000-0000-0000-000000000013"
            .parse()
            .expect("correlation header"),
    );
    request.headers_mut().insert(
        "x-ratatoskr-operation-id",
        "00000000-0000-0000-0000-000000000014"
            .parse()
            .expect("operation header"),
    );
    request.headers_mut().insert(
        "x-ratatoskr-archive-sha256",
        String::from("0").repeat(64).parse().expect("digest header"),
    );
    request.headers_mut().insert(
        "x-ratatoskr-archive-byte-size",
        total.to_string().parse().expect("size header"),
    );

    let (status, _) = send(&fixture.app, request).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(fixture.repository.exports_snapshot().is_empty());
}

/// Missing acquisition mode or missing media type answer invalid-request.
#[tokio::test]
async fn missing_acquisition_or_media_type_answer_400() {
    let fixture = fixture(u64::MAX);
    let (status, _) = send(
        &fixture.app,
        request(
            "POST",
            Some("Bearer tok-alpha"),
            None,
            Some("application/zip"),
            Some(12),
            byte_stream(GOOD_BODY.to_vec()),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "missing acquisition header"
    );

    let (status, _) = send(
        &fixture.app,
        request(
            "POST",
            Some("Bearer tok-alpha"),
            Some("consumer_export"),
            None,
            Some(12),
            byte_stream(GOOD_BODY.to_vec()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "missing media type");

    let (status, _) = send(
        &fixture.app,
        request(
            "POST",
            Some("Bearer tok-alpha"),
            Some("not_a_mode"),
            Some("application/zip"),
            Some(12),
            byte_stream(GOOD_BODY.to_vec()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "unknown acquisition value");
}
