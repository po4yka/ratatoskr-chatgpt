//! The pump publishes through its real connection function under the `ChatGPT` stanza of the
//! deployed ACL (XR-021 CONTRACTS.md R3-01). That stanza lets the identity subscribe only to
//! `_INBOX_chatgpt.>`, so a connection without the inbox prefix never receives the publish
//! acknowledgement and the pass times out.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "the isolated broker assertions are the integration contract"
)]

mod common;

use async_nats::jetstream;
use common::broker::PrivateBroker;
use ratatoskr_chatgpt_archive::receipt::ArchiveEventOutbox;

/// Creates what Edge creates: the events stream the `ChatGPT` identity may publish into.
async fn provision(broker: &PrivateBroker) {
    let administrator =
        async_nats::ConnectOptions::with_nkey(broker.administrator_seed().to_owned())
            .connect(broker.url())
            .await
            .expect("the provisioning identity connects");
    jetstream::new(administrator)
        .get_or_create_stream(jetstream::stream::Config {
            name: "ratatoskr_events".to_owned(),
            subjects: vec!["evt.>".to_owned()],
            ..jetstream::stream::Config::default()
        })
        .await
        .expect("the event stream exists");
}

#[tokio::test]
async fn the_pump_completes_jetstream_requests_under_its_inbox_prefix() {
    let broker = PrivateBroker::with_service_stanza();
    provision(&broker).await;
    let database = common::DisposableDatabase::create().await;
    common::outbox::enqueue_tombstone(database.pool()).await;

    let published = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        ArchiveEventOutbox::new(database.pool().clone())
            .publish_pending_once(&broker.url(), broker.service_seed_path()),
    )
    .await
    .expect("the publish acknowledgement arrives under the `ChatGPT` stanza")
    .expect("the pump publishes through its own connection");

    assert_eq!(published, 1, "the enqueued row is published");
    let marked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chatgpt_archive.outbox_events WHERE published_at IS NOT NULL",
    )
    .fetch_one(database.pool())
    .await
    .expect("the outbox can be counted");
    assert_eq!(marked, 1, "the acknowledged row is marked published");
    database.discard().await;
}
