//! The pump records the stream sequence of the broker acknowledgement of every row it publishes
//! (XR-021 CONTRACTS.md R5-12 rule 2). Every test runs the real pump against a private
//! `JetStream` broker.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test failures report through panics"
)]

mod common;

use async_nats::jetstream;
use common::broker::PrivateBroker;
use common::outbox::{enqueue_tombstone, seed_file};
use ratatoskr_chatgpt_archive::receipt::ArchiveEventOutbox;

async fn events_stream(broker: &PrivateBroker) -> jetstream::stream::Stream {
    let client = async_nats::connect(broker.url())
        .await
        .expect("the private broker accepts connections");
    jetstream::new(client)
        .create_stream(jetstream::stream::Config {
            name: "ratatoskr_events".to_owned(),
            subjects: vec!["evt.>".to_owned()],
            ..jetstream::stream::Config::default()
        })
        .await
        .expect("the events stream is created")
}

async fn recorded_sequence(pool: &sqlx::PgPool, event_id: &str) -> Option<i64> {
    sqlx::query_scalar(
        "SELECT published_stream_seq FROM chatgpt_archive.outbox_events
         WHERE payload ->> 'event_id' = $1",
    )
    .bind(event_id)
    .fetch_one(pool)
    .await
    .expect("the row is readable")
}

#[tokio::test]
async fn a_published_row_records_the_sequence_of_its_stream_message() {
    let broker = PrivateBroker::plain();
    let stream = events_stream(&broker).await;
    let database = common::DisposableDatabase::create().await;
    let first = enqueue_tombstone(database.pool()).await;
    let second = enqueue_tombstone(database.pool()).await;
    let seed = seed_file();

    let published = ArchiveEventOutbox::new(database.pool().clone())
        .publish_pending_once(&broker.url(), seed.path())
        .await
        .expect("the pump publishes");

    assert_eq!(published, 2);
    for event_id in [&first, &second] {
        let sequence = recorded_sequence(database.pool(), event_id)
            .await
            .expect("a published row records its stream sequence");
        let message = stream
            .get_raw_message(u64::try_from(sequence).expect("a positive sequence"))
            .await
            .expect("the recorded sequence names a stored message");
        assert_eq!(
            message
                .headers
                .get("Nats-Msg-Id")
                .map(ToString::to_string)
                .as_deref(),
            Some(event_id.as_str()),
            "the recorded sequence is the message this row published"
        );
    }
    database.discard().await;
}

#[tokio::test]
async fn a_duplicate_acknowledgement_records_the_original_sequence() {
    let broker = PrivateBroker::plain();
    let stream = events_stream(&broker).await;
    let database = common::DisposableDatabase::create().await;
    let event_id = enqueue_tombstone(database.pool()).await;
    let other = enqueue_tombstone(database.pool()).await;
    // The broker already holds the first row: a crash after the acknowledgement and before the
    // row was marked left it unpublished. Another message follows it.
    let body: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM chatgpt_archive.outbox_events WHERE payload ->> 'event_id' = $1",
    )
    .bind(&event_id)
    .fetch_one(database.pool())
    .await
    .expect("the row is readable");
    let client = async_nats::connect(broker.url()).await.expect("a client");
    let context = jetstream::new(client);
    let mut headers = async_nats::HeaderMap::new();
    headers.insert("Nats-Msg-Id", event_id.as_str());
    let original = context
        .publish_with_headers(
            "evt.ai_archive.subject.tombstoned.v1",
            headers,
            serde_json::to_vec(&body).expect("the body encodes").into(),
        )
        .await
        .expect("the publish is sent")
        .await
        .expect("the broker stores the original");
    context
        .publish("evt.ai_archive.subject.tombstoned.v1", "unrelated".into())
        .await
        .expect("the publish is sent")
        .await
        .expect("the broker stores another message");
    let seed = seed_file();

    ArchiveEventOutbox::new(database.pool().clone())
        .publish_pending_once(&broker.url(), seed.path())
        .await
        .expect("the pump publishes");

    assert_eq!(
        recorded_sequence(database.pool(), &event_id).await,
        Some(i64::try_from(original.sequence).expect("a small sequence")),
        "a duplicate acknowledgement carries the original sequence"
    );
    assert_ne!(
        recorded_sequence(database.pool(), &other).await,
        recorded_sequence(database.pool(), &event_id).await,
        "the next row got a sequence of its own"
    );
    drop(stream);
    database.discard().await;
}
