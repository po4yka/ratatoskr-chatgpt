//! The pump re-sends a published tombstone until Knowledge acknowledges it, with the same bytes
//! and the same `Nats-Msg-Id` (XR-021 CONTRACTS.md R5-11 rule 4). Every test runs the real pump
//! against a private `JetStream` broker.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test failures report through panics"
)]

mod common;

use std::time::Duration;

use async_nats::jetstream;
use common::acks::{acknowledgement, row_state};
use common::broker::PrivateBroker;
use common::outbox::{OWNER, enqueue_tombstone, seed_file};
use ratatoskr_chatgpt_archive::deletion_ack::{TombstoneResend, acknowledge};
use ratatoskr_chatgpt_archive::receipt::ArchiveEventOutbox;
use ratatoskr_identifiers::EventId;

/// The resend interval the tests inject. It must exceed the duplicate window of the test stream
/// below, or the broker would collapse the second copy.
const AFTER: Duration = Duration::from_millis(1_500);
const CAP: Duration = Duration::from_secs(3);

/// The events stream as Edge provisions it, except for a one second duplicate window instead of
/// the production 120 seconds, so a resend produces a second stream copy within a test.
async fn events_stream(broker: &PrivateBroker) -> jetstream::stream::Stream {
    let client = async_nats::connect(broker.url())
        .await
        .expect("the private broker accepts connections");
    jetstream::new(client)
        .create_stream(jetstream::stream::Config {
            name: "ratatoskr_events".to_owned(),
            subjects: vec!["evt.>".to_owned()],
            duplicate_window: Duration::from_secs(1),
            ..jetstream::stream::Config::default()
        })
        .await
        .expect("the events stream is created")
}

fn pump(pool: &sqlx::PgPool) -> ArchiveEventOutbox {
    ArchiveEventOutbox::new(pool.clone()).with_tombstone_resend(TombstoneResend {
        after: AFTER,
        cap: CAP,
    })
}

async fn pass(pump: &ArchiveEventOutbox, broker: &PrivateBroker, seed: &std::path::Path) -> usize {
    pump.publish_pending_once(&broker.url(), seed)
        .await
        .expect("the pump pass succeeds")
}

#[tokio::test]
async fn an_unacknowledged_tombstone_is_published_again_with_the_same_bytes_and_backs_off() {
    let broker = PrivateBroker::plain();
    let stream = events_stream(&broker).await;
    let database = common::DisposableDatabase::create().await;
    let event_id = EventId::parse(&enqueue_tombstone(database.pool()).await).expect("an event id");
    let seed = seed_file();
    let pump = pump(database.pool());

    assert_eq!(pass(&pump, &broker, seed.path()).await, 1, "first publish");
    let state = row_state(database.pool(), event_id).await;
    assert!(state.published && !state.acknowledged);
    assert_eq!(state.resend_count, 0);
    let wait = state.seconds_to_resend.expect("a resend is scheduled");
    assert!(
        (0.5..=1.6).contains(&wait),
        "the first resend waits about the interval, got {wait}"
    );
    assert_eq!(
        pass(&pump, &broker, seed.path()).await,
        0,
        "nothing is due before the interval passes"
    );

    tokio::time::sleep(AFTER + Duration::from_millis(200)).await;
    assert_eq!(pass(&pump, &broker, seed.path()).await, 1, "first resend");
    let state = row_state(database.pool(), event_id).await;
    assert_eq!(state.resend_count, 1);
    let wait = state
        .seconds_to_resend
        .expect("the next resend is scheduled");
    assert!(
        (2.0..=3.1).contains(&wait),
        "the second wait doubles to about 3 s, got {wait}"
    );

    tokio::time::sleep(CAP + Duration::from_millis(200)).await;
    assert_eq!(pass(&pump, &broker, seed.path()).await, 1, "second resend");
    let state = row_state(database.pool(), event_id).await;
    assert_eq!(state.resend_count, 2);
    let wait = state
        .seconds_to_resend
        .expect("the next resend is scheduled");
    assert!(
        wait <= 3.1,
        "the wait never exceeds the cap, got {wait} after the interval would have been 6 s"
    );

    let mut stream = stream;
    assert_eq!(
        stream.info().await.expect("stream info").state.messages,
        3,
        "the broker holds the first publish and both resends"
    );
    let mut ids = Vec::new();
    let mut bodies = Vec::new();
    for sequence in 1..=3 {
        let message = stream
            .get_raw_message(sequence)
            .await
            .expect("the stream holds the message");
        ids.push(
            message
                .headers
                .get("Nats-Msg-Id")
                .map(ToString::to_string)
                .expect("every copy carries the message id"),
        );
        bodies.push(message.payload);
    }
    assert!(
        ids.iter().all(|id| *id == event_id.to_string()),
        "every copy keeps the same Nats-Msg-Id: {ids:?}"
    );
    assert!(
        bodies.windows(2).all(|pair| pair[0] == pair[1]),
        "every copy carries the same bytes"
    );
    database.discard().await;
}

#[tokio::test]
async fn an_acknowledged_tombstone_is_never_published_again() {
    let broker = PrivateBroker::plain();
    let mut stream = events_stream(&broker).await;
    let database = common::DisposableDatabase::create().await;
    let event_id = EventId::parse(&enqueue_tombstone(database.pool()).await).expect("an event id");
    let seed = seed_file();
    let pump = pump(database.pool());
    assert_eq!(pass(&pump, &broker, seed.path()).await, 1);

    acknowledge(
        database.pool(),
        &acknowledgement(event_id, OWNER, "applied"),
    )
    .await
    .expect("the acknowledgement is recorded");
    tokio::time::sleep(AFTER + Duration::from_millis(300)).await;

    assert_eq!(
        pass(&pump, &broker, seed.path()).await,
        0,
        "an acknowledged row leaves the resend query"
    );
    assert_eq!(stream.info().await.expect("stream info").state.messages, 1);
    database.discard().await;
}

#[tokio::test]
async fn a_failed_republish_leaves_the_row_and_uses_the_failure_backoff() {
    let broker = PrivateBroker::plain();
    let _ = events_stream(&broker).await;
    let database = common::DisposableDatabase::create().await;
    let event_id = EventId::parse(&enqueue_tombstone(database.pool()).await).expect("an event id");
    let seed = seed_file();
    let pump = pump(database.pool());
    assert_eq!(pass(&pump, &broker, seed.path()).await, 1);
    // The stream disappears: the next publish finds no responder.
    let client = async_nats::connect(broker.url()).await.expect("a client");
    jetstream::new(client)
        .delete_stream("ratatoskr_events")
        .await
        .expect("the stream is deleted");
    tokio::time::sleep(AFTER + Duration::from_millis(200)).await;

    let failed = pump.publish_pending_once(&broker.url(), seed.path()).await;

    assert!(failed.is_err(), "the republish was not acknowledged");
    let state = row_state(database.pool(), event_id).await;
    assert!(state.published && !state.acknowledged, "the row is kept");
    assert_eq!(state.resend_count, 0, "a failed republish is not a resend");
    assert_eq!(state.attempt_count, 1, "the failure is recorded");
    let wait = state.seconds_to_resend.expect("a retry is scheduled");
    assert!(
        wait > 0.5,
        "the failure backoff pushes the next attempt out, got {wait}"
    );
    database.discard().await;
}
