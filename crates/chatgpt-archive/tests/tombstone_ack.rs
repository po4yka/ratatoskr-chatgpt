//! Recording Knowledge's acknowledgement of a tombstone, and what a later deletion keeps
//! (XR-021 CONTRACTS.md R5-11 rule 4).

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test failures report through panics"
)]

mod common;

use common::acks::{acknowledgement, row_state};
use common::outbox::{OWNER, enqueue_tombstone};
use common::{DisposableDatabase, blob, insert_account, insert_export};
use ratatoskr_chatgpt_archive::deletion_ack::{AckApplication, acknowledge};
use ratatoskr_chatgpt_archive::privacy_deletion::{PrivacyDeletionScope, PrivacyDeletionService};
use ratatoskr_chatgpt_archive::{BlobStore, NormalizedArchiveEvent, OutboxPlacement};
use ratatoskr_identifiers::EventId;
use uuid::Uuid;

/// Marks the row as the pump does after the broker acknowledged it.
async fn mark_published(database: &DisposableDatabase, event_id: EventId) {
    sqlx::query(
        "UPDATE chatgpt_archive.outbox_events
         SET published_at = now(), next_resend_at = now() + interval '10 minutes'
         WHERE payload ->> 'event_id' = $1",
    )
    .bind(event_id.to_string())
    .execute(database.pool())
    .await
    .expect("the row is marked published");
}

async fn published_tombstone(database: &DisposableDatabase) -> EventId {
    let event_id = enqueue_tombstone(database.pool()).await;
    let event_id = EventId::parse(&event_id).expect("a stored event id");
    mark_published(database, event_id).await;
    event_id
}

#[tokio::test]
async fn an_applied_acknowledgement_stops_the_resend() {
    let database = DisposableDatabase::create().await;
    let event_id = published_tombstone(&database).await;

    let outcome = acknowledge(
        database.pool(),
        &acknowledgement(event_id, OWNER, "applied"),
    )
    .await
    .expect("the acknowledgement is recorded");

    assert_eq!(outcome, AckApplication::Applied);
    let state = row_state(database.pool(), event_id).await;
    assert!(state.acknowledged, "acknowledged_at is set");
    assert_eq!(state.ack_outcome.as_deref(), Some("applied"));
    database.discard().await;
}

#[tokio::test]
async fn a_refused_acknowledgement_also_stops_the_resend() {
    let database = DisposableDatabase::create().await;
    let event_id = published_tombstone(&database).await;

    let outcome = acknowledge(
        database.pool(),
        &acknowledgement(event_id, OWNER, "refused"),
    )
    .await
    .expect("the acknowledgement is recorded");

    assert_eq!(outcome, AckApplication::Refused);
    let state = row_state(database.pool(), event_id).await;
    assert!(state.acknowledged, "a refusal ends the resend too");
    assert_eq!(state.ack_outcome.as_deref(), Some("refused"));
    database.discard().await;
}

#[tokio::test]
async fn an_acknowledgement_for_another_owner_or_an_unknown_id_changes_nothing() {
    let database = DisposableDatabase::create().await;
    let event_id = published_tombstone(&database).await;

    let other_owner = acknowledge(
        database.pool(),
        &acknowledgement(
            event_id,
            "user:018f0000-0000-7000-8000-0000000000ff",
            "applied",
        ),
    )
    .await
    .expect("an acknowledgement of another owner is ignored");
    let unknown = acknowledge(
        database.pool(),
        &acknowledgement(EventId::new_v7(), OWNER, "applied"),
    )
    .await
    .expect("an acknowledgement of an unknown id is ignored");

    assert_eq!(other_owner, AckApplication::Unmatched);
    assert_eq!(unknown, AckApplication::Unmatched);
    let state = row_state(database.pool(), event_id).await;
    assert!(!state.acknowledged, "the row is untouched");
    assert_eq!(state.ack_outcome, None);
    database.discard().await;
}

#[tokio::test]
async fn a_duplicate_acknowledgement_changes_nothing() {
    let database = DisposableDatabase::create().await;
    let event_id = published_tombstone(&database).await;
    acknowledge(
        database.pool(),
        &acknowledgement(event_id, OWNER, "applied"),
    )
    .await
    .expect("the first acknowledgement is recorded");
    let first = acknowledged_state(&database, event_id).await;

    let duplicate = acknowledge(
        database.pool(),
        &acknowledgement(event_id, OWNER, "refused"),
    )
    .await
    .expect("a duplicate is ignored");

    assert_eq!(duplicate, AckApplication::Unmatched);
    assert_eq!(
        acknowledged_state(&database, event_id).await,
        first,
        "neither the instant nor the outcome of the first acknowledgement changed"
    );
    database.discard().await;
}

/// The recorded instant and outcome, read as text so the test needs no time library.
async fn acknowledged_state(
    database: &DisposableDatabase,
    event_id: EventId,
) -> (Option<String>, Option<String>) {
    sqlx::query_as(
        "SELECT acknowledged_at::text, ack_outcome FROM chatgpt_archive.outbox_events
         WHERE payload ->> 'event_id' = $1",
    )
    .bind(event_id.to_string())
    .fetch_one(database.pool())
    .await
    .expect("the row is readable")
}

#[tokio::test]
async fn a_later_deletion_keeps_an_unacknowledged_tombstone_and_removes_an_acknowledged_one() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let tenant = insert_account(&database, "ack-keep").await;
    let raw = blob(&blobs, b"ack raw").await;
    insert_export(&database, tenant, Uuid::now_v7(), &raw).await;

    let mut event_ids = Vec::new();
    for subject in [
        r#"{"subject_kind":"archive"}"#,
        r#"{"subject_kind":"project","ai_project_id":"018f0000-0000-7000-8000-000000000404"}"#,
    ] {
        let event = NormalizedArchiveEvent::tombstoned(&common::outbox::tombstone(subject))
            .expect("a valid tombstone encodes");
        let mut transaction = database.pool().begin().await.expect("a transaction");
        ratatoskr_chatgpt_archive::Database::enqueue_normalized_event(
            &mut transaction,
            &event,
            &OutboxPlacement {
                account_id: Some(tenant),
                ..OutboxPlacement::default()
            },
        )
        .await
        .expect("the tombstone is enqueued");
        transaction.commit().await.expect("the commit succeeds");
        mark_published(&database, event.envelope.event_id).await;
        event_ids.push(event.envelope.event_id);
    }
    let [acknowledged, unacknowledged] = event_ids[..] else {
        panic!("two tombstones were enqueued");
    };
    acknowledge(
        database.pool(),
        &acknowledgement(acknowledged, OWNER, "applied"),
    )
    .await
    .expect("the acknowledgement is recorded");

    let service = PrivacyDeletionService::new(database.pool().clone(), blobs.clone());
    let request = Uuid::now_v7();
    service
        .plan(tenant, request, PrivacyDeletionScope::Tenant)
        .await
        .expect("planning succeeds")
        .expect("the owned tenant plans");
    service
        .execute(request)
        .await
        .expect("the tenant deletion completes");

    let remaining: Vec<String> = sqlx::query_scalar(
        "SELECT payload ->> 'event_id' FROM chatgpt_archive.outbox_events
         WHERE event_type = 'ai_archive.subject.tombstoned.v1' ORDER BY id",
    )
    .fetch_all(database.pool())
    .await
    .expect("the outbox is readable");
    assert_eq!(
        remaining,
        vec![unacknowledged.to_string()],
        "a published but unacknowledged tombstone is the only record that Knowledge must delete; \
         the acknowledged one is removed"
    );
    database.discard().await;
}
