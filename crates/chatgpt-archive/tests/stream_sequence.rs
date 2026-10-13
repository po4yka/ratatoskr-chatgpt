//! The pump records the stream sequence of every broker acknowledgement of every row it publishes,
//! in `published_stream_seqs`, ascending and without duplicates (XR-021 CONTRACTS.md R6-03 rule 1,
//! which replaces R5-12 rule 2). Every test runs the real pump against a private `JetStream`
//! broker.

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

/// The sequences the row records, as JSON, or none when the column does not exist or is null.
async fn recorded_sequences(pool: &sqlx::PgPool, event_id: &str) -> Option<serde_json::Value> {
    sqlx::query_scalar(
        "SELECT to_jsonb(o) -> 'published_stream_seqs' FROM chatgpt_archive.outbox_events o
         WHERE payload ->> 'event_id' = $1",
    )
    .bind(event_id)
    .fetch_one(pool)
    .await
    .expect("the row is readable")
}

/// The one sequence a row that was acknowledged once records.
async fn the_one_sequence(pool: &sqlx::PgPool, event_id: &str) -> i64 {
    let recorded = recorded_sequences(pool, event_id)
        .await
        .expect("a published row records its stream sequences");
    let sequences = recorded.as_array().expect("the column is an array");
    assert_eq!(sequences.len(), 1, "one acknowledgement, one element");
    sequences
        .first()
        .and_then(serde_json::Value::as_i64)
        .expect("a sequence is an integer")
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
        let sequence = the_one_sequence(database.pool(), event_id).await;
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
        the_one_sequence(database.pool(), &event_id).await,
        i64::try_from(original.sequence).expect("a small sequence"),
        "a duplicate acknowledgement carries the original sequence"
    );
    assert_ne!(
        the_one_sequence(database.pool(), &other).await,
        the_one_sequence(database.pool(), &event_id).await,
        "the next row got a sequence of its own"
    );
    drop(stream);
    database.discard().await;
}

#[tokio::test]
async fn a_second_publication_inside_the_duplicate_window_adds_no_element() {
    let broker = PrivateBroker::plain();
    let _stream = events_stream(&broker).await;
    let database = common::DisposableDatabase::create().await;
    let event_id = enqueue_tombstone(database.pool()).await;
    let seed = seed_file();
    let pump = ArchiveEventOutbox::new(database.pool().clone());
    pump.publish_pending_once(&broker.url(), seed.path())
        .await
        .expect("the pump publishes");
    let first = the_one_sequence(database.pool(), &event_id).await;
    // The mark of the first publication is taken back, as if it had never landed: the row is
    // published again under the same message id, inside the stream's two minute window.
    sqlx::query(
        "UPDATE chatgpt_archive.outbox_events
         SET published_at = NULL, published_stream_seqs = '{}'
         WHERE payload ->> 'event_id' = $1",
    )
    .bind(&event_id)
    .execute(database.pool())
    .await
    .expect("the mark is taken back");

    pump.publish_pending_once(&broker.url(), seed.path())
        .await
        .expect("the pump publishes again");

    assert_eq!(
        recorded_sequences(database.pool(), &event_id).await,
        Some(serde_json::json!([first])),
        "the duplicate acknowledgement carries the original sequence and adds nothing"
    );
    database.discard().await;
}
