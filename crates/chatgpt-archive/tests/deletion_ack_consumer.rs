//! The service consumes Knowledge's deletion acknowledgements from its Edge-provisioned durable
//! under the shipped `ChatGPT` ACL stanza (XR-021 CONTRACTS.md R5-11 rule 4), and the pump stops
//! re-sending the tombstone it names.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test failures report through panics"
)]

mod common;

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use async_nats::jetstream;
use common::acks::{acknowledgement, acknowledgement_envelope, row_state};
use common::broker::PrivateBroker;
use common::outbox::{OWNER, enqueue_tombstone};
use ratatoskr_chatgpt_archive::deletion_ack::TombstoneResend;
use ratatoskr_chatgpt_archive::deletion_ack::consumer::{
    AckSummary, DURABLE, EVENTS_STREAM, FILTER_SUBJECT, consume_deletion_acks,
};
use ratatoskr_chatgpt_archive::receipt::ArchiveEventOutbox;
use ratatoskr_identifiers::EventId;

const AFTER: Duration = Duration::from_millis(1_500);
const CAP: Duration = Duration::from_secs(3);

fn metrics() -> &'static metrics_exporter_prometheus::PrometheusHandle {
    static HANDLE: OnceLock<metrics_exporter_prometheus::PrometheusHandle> = OnceLock::new();
    HANDLE.get_or_init(|| {
        metrics_exporter_prometheus::PrometheusBuilder::new()
            .install_recorder()
            .expect("no other recorder is installed in this test binary")
    })
}

/// The value of a counter line of the Prometheus exposition, zero when it is not there yet.
fn counter(name: &str) -> f64 {
    metrics()
        .render()
        .lines()
        .find_map(|line| line.strip_prefix(name)?.trim().parse::<f64>().ok())
        .unwrap_or(0.0)
}

/// What Edge provisions: the events stream (with a one second duplicate window so a resend leaves
/// a second copy inside a test) and the durable of this service.
struct Edge {
    stream: jetstream::stream::Stream,
    context: jetstream::Context,
}

async fn provision(broker: &PrivateBroker) -> Edge {
    let client = async_nats::ConnectOptions::with_nkey(broker.administrator_seed().to_owned())
        .connect(broker.url())
        .await
        .expect("the provisioning identity connects");
    let context = jetstream::new(client);
    let stream = context
        .create_stream(jetstream::stream::Config {
            name: EVENTS_STREAM.to_owned(),
            subjects: vec!["evt.>".to_owned()],
            duplicate_window: Duration::from_secs(1),
            ..jetstream::stream::Config::default()
        })
        .await
        .expect("the events stream is created");
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
    Edge { stream, context }
}

impl Edge {
    /// Publishes `bytes` to the acknowledgement subject as the Knowledge identity would.
    async fn publish_acknowledgement(&self, bytes: Vec<u8>) {
        self.context
            .publish(FILTER_SUBJECT, bytes.into())
            .await
            .expect("the publish is sent")
            .await
            .expect("the broker stores the acknowledgement");
    }
}

struct Running {
    stop: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<Result<AckSummary, String>>,
}

fn start_consumer(broker: &PrivateBroker, pool: &sqlx::PgPool) -> Running {
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let (url, seed, pool) = (
        broker.url(),
        broker.service_seed_path().to_path_buf(),
        pool.clone(),
    );
    let task = tokio::spawn(async move {
        consume_deletion_acks(&url, &seed, &pool, async {
            let _ = stopped.await;
        })
        .await
        .map_err(|error| error.to_string())
    });
    Running { stop, task }
}

impl Running {
    async fn finish(self) -> AckSummary {
        let _ = self.stop.send(());
        self.task
            .await
            .expect("the consumer task joins")
            .expect("the consumer ends on the shutdown request")
    }
}

async fn wait_until<F, Fut>(what: &str, mut condition: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if condition().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}");
}

fn pump(pool: &sqlx::PgPool) -> ArchiveEventOutbox {
    ArchiveEventOutbox::new(pool.clone()).with_tombstone_resend(TombstoneResend {
        after: AFTER,
        cap: CAP,
    })
}

#[tokio::test]
async fn publish_no_acknowledgement_resend_acknowledgement_stop() {
    let broker = PrivateBroker::with_service_stanza();
    let mut edge = provision(&broker).await;
    let database = common::DisposableDatabase::create().await;
    let event_id = EventId::parse(&enqueue_tombstone(database.pool()).await).expect("an event id");
    let pump = pump(database.pool());
    let consumer = start_consumer(&broker, database.pool());

    let published = pump
        .publish_pending_once(&broker.url(), broker.service_seed_path())
        .await
        .expect("the first publish");
    eprintln!("transcript: published {published} tombstone, event id {event_id}");
    tokio::time::sleep(AFTER + Duration::from_millis(300)).await;
    let state = row_state(database.pool(), event_id).await;
    assert!(
        state.published && !state.acknowledged,
        "the first publish is not acknowledged"
    );
    eprintln!("transcript: no acknowledgement after {AFTER:?}, row unacknowledged");

    let republished = pump
        .publish_pending_once(&broker.url(), broker.service_seed_path())
        .await
        .expect("the resend");
    assert_eq!(republished, 1, "the same bytes are published again");
    let first = edge.stream.get_raw_message(1).await.expect("first copy");
    let second = edge.stream.get_raw_message(2).await.expect("second copy");
    assert_eq!(first.payload, second.payload);
    assert_eq!(
        first.headers.get("Nats-Msg-Id").map(ToString::to_string),
        second.headers.get("Nats-Msg-Id").map(ToString::to_string),
    );
    eprintln!(
        "transcript: resent, stream sequences 1 and 2 carry Nats-Msg-Id {} and equal bytes",
        second
            .headers
            .get("Nats-Msg-Id")
            .map(ToString::to_string)
            .unwrap_or_default()
    );

    let envelope = acknowledgement_envelope(&acknowledgement(event_id, OWNER, "applied"));
    edge.publish_acknowledgement(envelope.to_canonical_json().expect("encodes").into_bytes())
        .await;
    wait_until("the acknowledgement to be recorded", || async {
        row_state(database.pool(), event_id).await.acknowledged
    })
    .await;
    eprintln!("transcript: acknowledgement published by Knowledge, row acknowledged (applied)");

    tokio::time::sleep(CAP + Duration::from_millis(300)).await;
    let after_ack = pump
        .publish_pending_once(&broker.url(), broker.service_seed_path())
        .await
        .expect("the pass after the acknowledgement");
    assert_eq!(after_ack, 0, "the resend stopped");
    let summary = consumer.finish().await;
    assert_eq!(summary.applied, 1);
    let copies = edge
        .stream
        .info()
        .await
        .expect("stream info")
        .state
        .messages;
    assert_eq!(
        copies, 3,
        "two tombstone copies and the acknowledgement; none after the acknowledgement"
    );
    eprintln!("transcript: pass after the acknowledgement published {after_ack}; stopped");
    database.discard().await;
}

#[tokio::test]
async fn a_refused_acknowledgement_stops_the_resend_and_counts() {
    let broker = PrivateBroker::with_service_stanza();
    let edge = provision(&broker).await;
    let database = common::DisposableDatabase::create().await;
    let event_id = EventId::parse(&enqueue_tombstone(database.pool()).await).expect("an event id");
    pump(database.pool())
        .publish_pending_once(&broker.url(), broker.service_seed_path())
        .await
        .expect("the publish");
    let consumer = start_consumer(&broker, database.pool());
    let before = counter("chatgpt_archive_tombstone_refused_total");

    let envelope = acknowledgement_envelope(&acknowledgement(event_id, OWNER, "refused"));
    edge.publish_acknowledgement(envelope.to_canonical_json().expect("encodes").into_bytes())
        .await;
    wait_until("the refusal to be recorded", || async {
        row_state(database.pool(), event_id).await.acknowledged
    })
    .await;

    let state = row_state(database.pool(), event_id).await;
    assert_eq!(state.ack_outcome.as_deref(), Some("refused"));
    assert!(
        (counter("chatgpt_archive_tombstone_refused_total") - before - 1.0).abs() < f64::EPSILON,
        "the refusal is counted once"
    );
    assert_eq!(consumer.finish().await.refused, 1);
    database.discard().await;
}

#[tokio::test]
async fn an_unknown_id_is_ignored_and_an_undecodable_message_is_terminated() {
    let broker = PrivateBroker::with_service_stanza();
    let edge = provision(&broker).await;
    let database = common::DisposableDatabase::create().await;
    let event_id = EventId::parse(&enqueue_tombstone(database.pool()).await).expect("an event id");
    let consumer = start_consumer(&broker, database.pool());

    // An acknowledgement for no row of this service.
    let unknown = acknowledgement_envelope(&acknowledgement(EventId::new_v7(), OWNER, "applied"));
    edge.publish_acknowledgement(unknown.to_canonical_json().expect("encodes").into_bytes())
        .await;
    // Bytes that are not an envelope.
    edge.publish_acknowledgement(b"not an envelope".to_vec())
        .await;
    // A valid envelope from the wrong producer.
    let mut forged = acknowledgement_envelope(&acknowledgement(event_id, OWNER, "applied"));
    forged.producer =
        ratatoskr_event_envelope::ProducerName::parse("ratatoskr-claude").expect("a producer");
    edge.publish_acknowledgement(forged.to_canonical_json().expect("encodes").into_bytes())
        .await;

    let mut durable = edge
        .stream
        .get_consumer::<jetstream::consumer::pull::Config>(DURABLE)
        .await
        .expect("the durable exists");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let info = durable.info().await.expect("consumer info");
        if info.num_pending == 0 && info.num_ack_pending == 0 && info.ack_floor.stream_sequence >= 3
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "every message must leave the durable's pending set, got {info:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let state = row_state(database.pool(), event_id).await;
    assert!(
        !state.acknowledged,
        "a forged acknowledgement from another producer changes nothing"
    );
    let summary = consumer.finish().await;
    assert_eq!(
        summary.ignored, 1,
        "the unknown id is acknowledged and ignored"
    );
    assert_eq!(
        summary.terminated, 2,
        "the garbage and the forgery are terminated"
    );
    database.discard().await;
}
