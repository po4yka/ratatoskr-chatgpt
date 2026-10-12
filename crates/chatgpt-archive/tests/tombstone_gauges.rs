//! The unacknowledged-tombstone gauges (XR-021 CONTRACTS.md R5-11 rule 4). They are refreshed by
//! every pump pass, even a pass with nothing to publish, and readiness is not tied to them.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test failures report through panics"
)]

mod common;

use common::acks::acknowledgement;
use common::outbox::{OWNER, enqueue_tombstone, seed_file};
use ratatoskr_chatgpt_archive::deletion_ack::acknowledge;
use ratatoskr_chatgpt_archive::receipt::ArchiveEventOutbox;
use ratatoskr_identifiers::EventId;

fn gauge(handle: &metrics_exporter_prometheus::PrometheusHandle, name: &str) -> f64 {
    handle
        .render()
        .lines()
        .find_map(|line| line.strip_prefix(name)?.trim().parse::<f64>().ok())
        .unwrap_or(f64::NAN)
}

#[tokio::test]
async fn the_gauges_follow_the_unacknowledged_tombstones() {
    let handle = metrics_exporter_prometheus::PrometheusBuilder::new()
        .install_recorder()
        .expect("no other recorder is installed in this test binary");
    let database = common::DisposableDatabase::create().await;
    let first = EventId::parse(&enqueue_tombstone(database.pool()).await).expect("an event id");
    let _second = enqueue_tombstone(database.pool()).await;
    // Both are published and not due for a resend, so the pass has nothing to publish and makes
    // no network call.
    sqlx::query(
        "UPDATE chatgpt_archive.outbox_events
         SET published_at = now(), next_resend_at = now() + interval '1 hour',
             created_at = now() - interval '90 seconds'",
    )
    .execute(database.pool())
    .await
    .expect("the rows are marked published");
    let seed = seed_file();
    let pump = ArchiveEventOutbox::new(database.pool().clone());

    pump.publish_pending_once("nats://127.0.0.1:1", seed.path())
        .await
        .expect("a pass with nothing due never touches the network");

    assert!(
        (gauge(&handle, "chatgpt_archive_unacknowledged_tombstones") - 2.0).abs() < f64::EPSILON,
        "two tombstones wait for Knowledge"
    );
    let oldest = gauge(
        &handle,
        "chatgpt_archive_oldest_unacknowledged_tombstone_age_seconds",
    );
    assert!(
        (85.0..=200.0).contains(&oldest),
        "the oldest unacknowledged tombstone is about 90 seconds old, got {oldest}"
    );

    acknowledge(database.pool(), &acknowledgement(first, OWNER, "applied"))
        .await
        .expect("the acknowledgement is recorded");
    pump.publish_pending_once("nats://127.0.0.1:1", seed.path())
        .await
        .expect("the next pass");
    assert!(
        (gauge(&handle, "chatgpt_archive_unacknowledged_tombstones") - 1.0).abs() < f64::EPSILON,
        "an acknowledged tombstone leaves the count"
    );
    database.discard().await;
}
