//! A fact row in flight is serialized with its deletion, and every published copy of a deleted
//! fact gets an erasure command (XR-021 CONTRACTS.md R6-03). Every test runs the real pump, the
//! real deletion service and a real `JetStream` broker with streams shaped like the deployed
//! `ratatoskr_events` and `ratatoskr_commands`; a stand-in for Edge's handler executes the
//! commands the pump publishes.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test failures report through panics"
)]

mod common;

use std::time::Duration;

use async_nats::jetstream;
use common::broker::PrivateBroker;
use common::imports::Fixture;
use common::outbox::seed_file;
use ratatoskr_chatgpt_archive::privacy_deletion::PrivacyDeletionScope;
use ratatoskr_chatgpt_archive::receipt::{
    ArchiveEventOutbox, PublishError, PublishPause, PublishProbe,
};
use ratatoskr_event_envelope::CommandEnvelope;
use ratatoskr_operation_contracts::PlatformStreamErasureRequested;
use uuid::Uuid;

const USER_A: &str = "018f0000-0000-7000-8000-0000000000a1";
const USER_B: &str = "018f0000-0000-7000-8000-0000000000b2";
const COMMAND_TYPE: &str = "platform.stream_erasure.requested.v1";
const TOMBSTONE_SUBJECT: &str = "evt.ai_archive.subject.tombstoned.v1";

fn owner(user: &str) -> String {
    format!("user:{user}")
}

struct Streams {
    events: jetstream::stream::Stream,
    commands: jetstream::stream::Stream,
}

/// The events and commands streams as Edge provisions them, with the given duplicate window.
async fn streams(broker: &PrivateBroker, duplicate_window: Duration) -> Streams {
    let client = async_nats::connect(broker.url())
        .await
        .expect("the private broker accepts connections");
    let context = jetstream::new(client);
    let events = context
        .create_stream(jetstream::stream::Config {
            name: "ratatoskr_events".to_owned(),
            subjects: vec!["evt.>".to_owned()],
            duplicate_window,
            ..jetstream::stream::Config::default()
        })
        .await
        .expect("the events stream is created");
    let commands = context
        .create_stream(jetstream::stream::Config {
            name: "ratatoskr_commands".to_owned(),
            subjects: vec!["cmd.>".to_owned()],
            ..jetstream::stream::Config::default()
        })
        .await
        .expect("the commands stream is created");
    Streams { events, commands }
}

/// The sequences of the content-bearing fact messages on the events stream that belong to
/// `owner`: every `evt.ai_archive.*` message except the tombstone, whose envelope names the owner.
async fn fact_sequences(events: &jetstream::stream::Stream, owner: &str) -> Vec<u64> {
    let last = events
        .get_info()
        .await
        .expect("the stream info is readable")
        .state
        .last_sequence;
    let mut found = Vec::new();
    for sequence in 1..=last {
        let Ok(message) = events.get_raw_message(sequence).await else {
            continue;
        };
        let subject = message.subject.to_string();
        if !subject.starts_with("evt.ai_archive.") || subject == TOMBSTONE_SUBJECT {
            continue;
        }
        let envelope: serde_json::Value =
            serde_json::from_slice(&message.payload).expect("a fact message is JSON");
        if envelope
            .get("tenant_id")
            .and_then(serde_json::Value::as_str)
            == Some(owner)
        {
            found.push(sequence);
        }
    }
    found
}

struct Command {
    envelope: CommandEnvelope,
    payload: PlatformStreamErasureRequested,
}

async fn commands(pool: &sqlx::PgPool) -> Vec<Command> {
    let rows: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT payload FROM chatgpt_archive.outbox_events
         WHERE event_type = $1 ORDER BY id",
    )
    .bind(COMMAND_TYPE)
    .fetch_all(pool)
    .await
    .expect("the outbox is readable");
    rows.into_iter()
        .map(|stored| {
            let envelope: CommandEnvelope =
                serde_json::from_value(stored).expect("a complete command envelope");
            let payload = envelope
                .payload_as::<PlatformStreamErasureRequested>()
                .expect("the payload is the erasure request");
            Command { envelope, payload }
        })
        .collect()
}

/// The stand-in for Edge's handler: executes every command on the commands stream by erasing the
/// stream sequences it names from the events stream.
async fn execute_commands(streams: &Streams) {
    let last = streams
        .commands
        .get_info()
        .await
        .expect("the stream info is readable")
        .state
        .last_sequence;
    for sequence in 1..=last {
        let message = streams
            .commands
            .get_raw_message(sequence)
            .await
            .expect("a command is stored");
        let envelope = CommandEnvelope::from_json(&message.payload).expect("a command envelope");
        let request = envelope
            .payload_as::<PlatformStreamErasureRequested>()
            .expect("the erasure request");
        for named in &request.stream_sequences {
            // A message already gone is skipped, like the real handler does.
            let _ = streams.events.delete_message(*named).await;
        }
    }
}

/// One pump pass in its own task, so a test can act while the pump is held.
fn spawn_pass(
    pump: &ArchiveEventOutbox,
    broker: &PrivateBroker,
    seed: &tempfile::NamedTempFile,
) -> tokio::task::JoinHandle<Result<usize, PublishError>> {
    let pump = pump.clone();
    let url = broker.url();
    let path = seed.path().to_owned();
    tokio::spawn(async move { pump.publish_pending_once(&url, &path).await })
}

#[tokio::test]
async fn a_fact_deleted_before_the_lock_is_never_published() {
    let broker = PrivateBroker::plain();
    let streams = streams(&broker, Duration::from_mins(2)).await;
    let fixture = Fixture::new().await;
    let received = fixture
        .import(Some(USER_A), &[("conv-a", "secret of a")])
        .await;
    let probe = PublishProbe::pausing(PublishPause::AfterCandidates);
    let pump =
        ArchiveEventOutbox::new(fixture.database.pool().clone()).with_publish_probe(probe.clone());
    let seed = seed_file();
    let pass = spawn_pass(&pump, &broker, &seed);
    probe.reached().await;

    // The pump chose its candidates; the deletion commits before it locks the first row.
    fixture
        .delete(
            received.account,
            PrivacyDeletionScope::Archive {
                ai_archive_id: received.archive,
            },
        )
        .await;
    probe.resume();
    let outcome = pass.await.expect("the pass task does not panic");

    assert_eq!(
        fact_sequences(&streams.events, &owner(USER_A)).await,
        Vec::<u64>::new(),
        "no message of the deleted facts reached the stream"
    );
    assert!(
        commands(fixture.database.pool()).await.is_empty(),
        "nothing was published, so nothing needs erasing"
    );
    outcome.expect("skipping a deleted row is not a failure");
    fixture.database.discard().await;
}

#[tokio::test]
async fn a_deletion_inside_the_acknowledgement_window_waits_and_erases_the_copy() {
    let broker = PrivateBroker::plain();
    let streams = streams(&broker, Duration::from_mins(2)).await;
    let fixture = Fixture::new().await;
    let first = fixture
        .import(Some(USER_A), &[("conv-a", "secret of a")])
        .await;
    fixture
        .import(Some(USER_B), &[("conv-x", "secret of b")])
        .await;
    let probe = PublishProbe::pausing(PublishPause::AfterPubAck);
    let pump =
        ArchiveEventOutbox::new(fixture.database.pool().clone()).with_publish_probe(probe.clone());
    let seed = seed_file();
    let pass = spawn_pass(&pump, &broker, &seed);
    probe.reached().await;
    let acknowledged = streams
        .events
        .get_info()
        .await
        .expect("the stream info is readable")
        .state
        .last_sequence;

    // The broker acknowledged the first fact and the pump has not committed its sequence.
    let request = Uuid::now_v7();
    let service = fixture.service();
    service
        .plan(
            first.account,
            request,
            PrivacyDeletionScope::Archive {
                ai_archive_id: first.archive,
            },
        )
        .await
        .expect("planning succeeds")
        .expect("the owned archive plans");
    let mut deletion = tokio::spawn(async move { service.execute(request).await });
    assert!(
        tokio::time::timeout(Duration::from_millis(500), &mut deletion)
            .await
            .is_err(),
        "the deletion waits for the row the pump holds"
    );
    probe.resume();
    pass.await
        .expect("the pass task does not panic")
        .expect("the pass publishes");
    deletion
        .await
        .expect("the deletion task does not panic")
        .expect("the deletion completes");

    let published_for_a = fact_sequences(&streams.events, &owner(USER_A)).await;
    assert!(
        published_for_a.contains(&acknowledged),
        "the acknowledged copy is on the stream"
    );
    let requested = commands(fixture.database.pool()).await;
    assert_eq!(requested.len(), 1, "one command for the one owner");
    assert_eq!(requested[0].payload.owner.to_string(), owner(USER_A));
    assert_eq!(
        requested[0].payload.stream_sequences, published_for_a,
        "the command names the acknowledged sequence and every other copy of the owner"
    );
    pump.publish_pending_once(&broker.url(), seed.path())
        .await
        .expect("the pump publishes the command");
    execute_commands(&streams).await;
    assert_eq!(
        fact_sequences(&streams.events, &owner(USER_A)).await,
        Vec::<u64>::new(),
        "every copy of the deleted facts is gone from the stream"
    );
    assert!(
        !fact_sequences(&streams.events, &owner(USER_B))
            .await
            .is_empty(),
        "another owner's facts are untouched"
    );
    fixture.database.discard().await;
}

/// The row id and event id of the first unpublished content-bearing fact of the outbox.
async fn first_fact(pool: &sqlx::PgPool) -> (i64, String) {
    sqlx::query_as(
        "SELECT id, payload ->> 'event_id' FROM chatgpt_archive.outbox_events
         WHERE event_type LIKE 'ai_archive.%' AND event_type <> 'ai_archive.subject.tombstoned.v1'
         ORDER BY id LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("a fact row exists")
}

async fn recorded(pool: &sqlx::PgPool, id: i64) -> Vec<i64> {
    sqlx::query_scalar(
        "SELECT published_stream_seqs FROM chatgpt_archive.outbox_events WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .expect("the row is readable")
}

#[tokio::test]
async fn a_republish_after_the_duplicate_window_records_both_sequences() {
    let broker = PrivateBroker::plain();
    let streams = streams(&broker, Duration::from_secs(1)).await;
    let fixture = Fixture::new().await;
    let received = fixture
        .import(Some(USER_A), &[("conv-a", "secret of a")])
        .await;
    let (id, _) = first_fact(fixture.database.pool()).await;
    let probe = PublishProbe::failing_commit_once();
    let pump =
        ArchiveEventOutbox::new(fixture.database.pool().clone()).with_publish_probe(probe.clone());
    let seed = seed_file();

    pump.publish_pending_once(&broker.url(), seed.path())
        .await
        .expect_err("the commit fails after the broker acknowledged");
    assert!(
        recorded(fixture.database.pool(), id).await.is_empty(),
        "the failed commit recorded nothing"
    );
    tokio::time::sleep(Duration::from_millis(2500)).await;
    pump.publish_pending_once(&broker.url(), seed.path())
        .await
        .expect("the retry publishes the row again");

    let both = recorded(fixture.database.pool(), id).await;
    assert_eq!(both.len(), 2, "both copies are recorded: {both:?}");
    assert!(both[0] < both[1], "ascending");
    fixture
        .delete(
            received.account,
            PrivacyDeletionScope::Archive {
                ai_archive_id: received.archive,
            },
        )
        .await;
    let requested = commands(fixture.database.pool()).await;
    assert_eq!(requested.len(), 1, "one command for the one owner");
    let named = &requested[0].payload.stream_sequences;
    for sequence in &both {
        let sequence = u64::try_from(*sequence).expect("a positive sequence");
        assert!(
            named.contains(&sequence),
            "{sequence} is named in {named:?}"
        );
    }
    pump.publish_pending_once(&broker.url(), seed.path())
        .await
        .expect("the pump publishes the command");
    execute_commands(&streams).await;
    assert_eq!(
        fact_sequences(&streams.events, &owner(USER_A)).await,
        Vec::<u64>::new(),
        "neither copy is left on the stream"
    );
    fixture.database.discard().await;
}

#[tokio::test]
async fn a_carried_copy_of_a_deleted_row_is_erased_by_the_pump() {
    let broker = PrivateBroker::plain();
    let streams = streams(&broker, Duration::from_secs(1)).await;
    let fixture = Fixture::new().await;
    let received = fixture
        .import(Some(USER_A), &[("conv-a", "secret of a")])
        .await;
    let (_, event_id) = first_fact(fixture.database.pool()).await;
    let probe = PublishProbe::failing_commit_once();
    let pump =
        ArchiveEventOutbox::new(fixture.database.pool().clone()).with_publish_probe(probe.clone());
    let seed = seed_file();
    pump.publish_pending_once(&broker.url(), seed.path())
        .await
        .expect_err("the commit fails after the broker acknowledged");
    let copy = fact_sequences(&streams.events, &owner(USER_A)).await;
    assert_eq!(copy.len(), 1, "the broker holds the one acknowledged copy");

    // The deletion finds the row unpublished, so it has no sequence to name.
    fixture
        .delete(
            received.account,
            PrivacyDeletionScope::Archive {
                ai_archive_id: received.archive,
            },
        )
        .await;
    assert!(
        commands(fixture.database.pool()).await.is_empty(),
        "no deletion knew about the copy"
    );
    pump.publish_pending_once(&broker.url(), seed.path())
        .await
        .expect("the next pass reconciles and publishes");

    let requested = commands(fixture.database.pool()).await;
    assert_eq!(requested.len(), 1, "the pump enqueued one command itself");
    assert_eq!(requested[0].payload.owner.to_string(), owner(USER_A));
    assert_eq!(requested[0].payload.stream_sequences, copy);
    assert_eq!(
        requested[0].payload.request_ref.to_wire(),
        format!("privacy_request:{event_id}"),
        "the request reference is the event id of the orphaned row"
    );
    assert_eq!(
        requested[0].envelope.aggregate_id,
        requested[0].payload.request_ref
    );
    pump.publish_pending_once(&broker.url(), seed.path())
        .await
        .expect("a later pass");
    assert_eq!(
        commands(fixture.database.pool()).await.len(),
        1,
        "a second pass enqueues nothing more"
    );
    execute_commands(&streams).await;
    assert_eq!(
        fact_sequences(&streams.events, &owner(USER_A)).await,
        Vec::<u64>::new(),
        "the carried copy is erased"
    );
    fixture.database.discard().await;
}
