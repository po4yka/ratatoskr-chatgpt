//! The service consumes Knowledge's deletion acknowledgements from its Edge-provisioned durable
//! and stays ready while Knowledge never answers (XR-021 CONTRACTS.md R5-11 rule 4). The service
//! runs as a child process against a private broker whose only service user is the shipped
//! `deploy/nats/identity.conf` stanza, exactly as it is deployed.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test failures report through panics"
)]

#[path = "../../../crates/chatgpt-archive/tests/common/broker.rs"]
#[allow(
    dead_code,
    reason = "the shared private-broker helper offers more than this test uses"
)]
mod broker;

use std::io::{Read as _, Write as _};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use async_nats::jetstream;
use broker::PrivateBroker;
use ratatoskr_chatgpt_archive::deletion_ack::consumer::{DURABLE, EVENTS_STREAM, FILTER_SUBJECT};
use ratatoskr_chatgpt_archive::{Database, NormalizedArchiveEvent, OutboxPlacement};
use ratatoskr_event_envelope::{
    EnvelopeSchemaVersion, EventEnvelope, EventPayload as _, ProducerName,
};
use ratatoskr_identifiers::{EventId, Extensions, WireTimestamp};
use ratatoskr_operation_contracts::KnowledgeDeletionAcknowledged;

const OWNER: &str = "user:018f0000-0000-7000-8000-000000000005";

struct Service(Child);

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn get(port: u16, target: &str) -> Option<(u16, String)> {
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let head = format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    stream.write_all(head.as_bytes()).ok()?;
    let mut text = String::new();
    stream.read_to_string(&mut text).ok()?;
    let code = text.split_whitespace().nth(1)?.parse().ok()?;
    let body = text.split_once("\r\n\r\n")?.1.to_owned();
    Some((code, body))
}

async fn eventually<F: FnMut() -> bool>(what: &str, mut condition: F) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("timed out waiting for {what}");
}

fn ready(port: u16) -> Option<bool> {
    get(port, "/health/ready").map(|(code, _)| code == 200)
}

fn gauge(port: u16, name: &str) -> Option<f64> {
    let (_, body) = get(port, "/metrics")?;
    body.lines()
        .find_map(|line| line.strip_prefix(name)?.trim().parse::<f64>().ok())
}

#[allow(
    clippy::disallowed_methods,
    reason = "the harness reads the PostgreSQL URL the runner exports"
)]
fn administrative_url() -> String {
    std::env::var("CHATGPT_TEST_DATABASE_URL")
        .ok()
        .filter(|url| !url.trim().is_empty())
        .expect("CHATGPT_TEST_DATABASE_URL must name a PostgreSQL for this test")
}

/// What Edge provisions before the service starts, except the durable.
async fn edge_stream(broker: &PrivateBroker) -> (jetstream::Context, jetstream::stream::Stream) {
    let administrator =
        async_nats::ConnectOptions::with_nkey(broker.administrator_seed().to_owned())
            .connect(broker.url())
            .await
            .expect("the provisioning identity connects");
    let context = jetstream::new(administrator);
    let stream = context
        .create_stream(jetstream::stream::Config {
            name: EVENTS_STREAM.to_owned(),
            subjects: vec!["evt.>".to_owned()],
            ..jetstream::stream::Config::default()
        })
        .await
        .expect("the events stream exists");
    (context, stream)
}

/// A disposable database of its own, so the gauges count only this test's tombstones.
async fn disposable_database() -> (sqlx::PgPool, String, String) {
    let admin_url = administrative_url();
    let admin = sqlx::PgPool::connect(&admin_url).await.expect("admin pool");
    let name = format!("xr021_svc_{}", uuid::Uuid::now_v7().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .expect("a disposable database");
    let (server, _) = admin_url.rsplit_once('/').expect("a database URL");
    (admin, name.clone(), format!("{server}/{name}"))
}

fn start_service(
    broker: &PrivateBroker,
    database_url: &str,
    storage: &std::path::Path,
) -> (Service, u16) {
    let admin_port = {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a free port");
        listener.local_addr().expect("an address").port()
    };
    let child = Command::new(env!("CARGO_BIN_EXE_ratatoskr-chatgpt-archive"))
        .env(
            "RATATOSKR__ADMIN__LISTEN_ADDRESS",
            format!("127.0.0.1:{admin_port}"),
        )
        .env("RATATOSKR__STORAGE__DATABASE_URL", database_url)
        .env("RATATOSKR__STORAGE__BLOB_ROOT", storage.join("blobs"))
        .env("RATATOSKR__RECEIPT__EVENT_BUS_URL", broker.url())
        .env(
            "RATATOSKR__RECEIPT__EVENT_BUS_NKEY_SEED_PATH",
            broker.service_seed_path(),
        )
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("the service starts");
    (Service(child), admin_port)
}

fn tombstone() -> NormalizedArchiveEvent {
    NormalizedArchiveEvent::tombstoned(
        &serde_json::from_value(serde_json::json!({
            "ai_archive_id": "018f0000-0000-7000-8000-000000000402",
            "provider": "chatgpt",
            "owner": OWNER,
            "subject": {"subject_kind": "archive"},
            "reason": "user_requested",
            "evidence_ref": {
                "owner_service": "ratatoskr-chatgpt",
                "digest": {"algorithm": "sha256", "hex": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},
                "media_type": "application/json",
                "length_bytes": 512
            },
            "observed_at": "2026-08-27T06:00:00Z"
        }))
        .expect("a literal tombstone"),
    )
    .expect("a valid tombstone encodes")
}

/// The envelope Knowledge publishes to acknowledge `event` as applied.
fn applied_acknowledgement(event: &NormalizedArchiveEvent) -> EventEnvelope {
    let acknowledgement: KnowledgeDeletionAcknowledged =
        serde_json::from_value(serde_json::json!({
            "acknowledged_event_id": event.envelope.event_id.to_string(),
            "family": "ai_archive",
            "owner": OWNER,
            "subject": "ai_archive:018f0000-0000-7000-8000-000000000402",
            "outcome": "applied",
            "acknowledged_at": "2026-10-12T10:00:00Z",
        }))
        .expect("a literal acknowledgement");
    let serde_json::Value::Object(payload) =
        serde_json::to_value(&acknowledgement).expect("the acknowledgement encodes")
    else {
        panic!("an acknowledgement is a JSON object");
    };
    let id = EventId::new_v7();
    EventEnvelope {
        event_id: id,
        event_type: KnowledgeDeletionAcknowledged::event_type(),
        occurred_at: WireTimestamp::parse("2026-10-12T10:00:00Z").expect("a literal instant"),
        producer: ProducerName::parse("ratatoskr-knowledge").expect("a producer"),
        aggregate_id: acknowledgement.subject.clone(),
        correlation_id: id.as_entity_ref(),
        causation_id: Some(acknowledgement.acknowledged_event_id.as_entity_ref()),
        tenant_id: Some(acknowledgement.owner),
        schema_version: EnvelopeSchemaVersion::CURRENT,
        payload,
        extensions: Extensions::new(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_service_waits_for_the_durable_consumes_acknowledgements_and_stays_ready_unanswered() {
    let broker = PrivateBroker::with_service_stanza();
    let (context, stream) = edge_stream(&broker).await;
    let (admin, name, database_url) = disposable_database().await;
    let storage = tempfile::tempdir().expect("storage");
    let (service, admin_port) = start_service(&broker, &database_url, storage.path());

    eventually("the operator listener", || {
        get(admin_port, "/health/live").is_some()
    })
    .await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        ready(admin_port),
        Some(false),
        "the service is not ready while Edge has not provisioned its durable"
    );

    stream
        .create_consumer(jetstream::consumer::pull::Config {
            durable_name: Some(DURABLE.to_owned()),
            filter_subject: FILTER_SUBJECT.to_owned(),
            ack_policy: jetstream::consumer::AckPolicy::Explicit,
            ack_wait: Duration::from_secs(30),
            ..jetstream::consumer::pull::Config::default()
        })
        .await
        .expect("the durable is created");
    eventually("the service to find the durable and report ready", || {
        ready(admin_port) == Some(true)
    })
    .await;

    // A tombstone enters the outbox; the pump publishes it and nothing ever answers.
    let pool = sqlx::PgPool::connect(&database_url)
        .await
        .expect("service pool");
    let event = tombstone();
    let mut transaction = pool.begin().await.expect("a transaction");
    Database::enqueue_normalized_event(&mut transaction, &event, &OutboxPlacement::default())
        .await
        .expect("the tombstone is enqueued");
    transaction.commit().await.expect("the commit succeeds");
    eventually("the gauge to show one unacknowledged tombstone", || {
        gauge(admin_port, "chatgpt_archive_unacknowledged_tombstones") == Some(1.0)
    })
    .await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        ready(admin_port),
        Some(true),
        "the service stays ready while Knowledge never answers"
    );
    let published: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chatgpt_archive.outbox_events WHERE published_at IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("the outbox is readable");
    assert_eq!(published, 1, "the pump published the tombstone");

    // Knowledge answers: the consumer records it and the gauge falls.
    context
        .publish(
            FILTER_SUBJECT,
            applied_acknowledgement(&event)
                .to_canonical_json()
                .expect("encodes")
                .into_bytes()
                .into(),
        )
        .await
        .expect("the publish is sent")
        .await
        .expect("the broker stores the acknowledgement");
    eventually("the acknowledgement to be consumed", || {
        gauge(admin_port, "chatgpt_archive_unacknowledged_tombstones") == Some(0.0)
    })
    .await;
    assert_eq!(ready(admin_port), Some(true));

    drop(service);
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .expect("the disposable database is dropped");
}
