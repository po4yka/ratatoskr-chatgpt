//! A deletion asks Edge to erase the stream copies of the published facts it removed from the
//! outbox (XR-021 CONTRACTS.md R5-12 rule 3). The first tests drive the real import and deletion
//! against `PostgreSQL`, marking facts published the way the pump does; the last runs the real pump
//! against a private `JetStream` broker and a fake of the Platform handler.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test failures report through panics"
)]

mod common;

use async_nats::jetstream;
use common::broker::PrivateBroker;
use common::imports::Fixture;
use common::outbox::seed_file;
use ratatoskr_chatgpt_archive::privacy_deletion::PrivacyDeletionScope;
use ratatoskr_chatgpt_archive::receipt::ArchiveEventOutbox;
use ratatoskr_event_envelope::CommandEnvelope;
use ratatoskr_operation_contracts::PlatformStreamErasureRequested;
use uuid::Uuid;

const USER_A: &str = "018f0000-0000-7000-8000-0000000000a1";
const USER_B: &str = "018f0000-0000-7000-8000-0000000000b2";
const COMMAND_TYPE: &str = "platform.stream_erasure.requested.v1";

fn owner(user: &str) -> String {
    format!("user:{user}")
}

/// Marks every unpublished fact, operation report and tombstone published with the next stream
/// sequences, in id order, as the pump records them. Returns `(outbox id, sequence)`.
async fn publish_as_the_pump_does(pool: &sqlx::PgPool) -> Vec<(i64, i64)> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM chatgpt_archive.outbox_events
         WHERE published_at IS NULL AND event_type <> $1 ORDER BY id",
    )
    .bind(COMMAND_TYPE)
    .fetch_all(pool)
    .await
    .expect("the outbox is readable");
    let base: i64 = sqlx::query_scalar(
        "SELECT COALESCE(max(sequence), 0)
         FROM chatgpt_archive.outbox_events, unnest(published_stream_seqs) AS sequence",
    )
    .fetch_one(pool)
    .await
    .expect("the outbox is readable");
    let mut published = Vec::new();
    for (offset, id) in ids.into_iter().enumerate() {
        let sequence = base + 1 + i64::try_from(offset).expect("a small offset");
        sqlx::query(
            "UPDATE chatgpt_archive.outbox_events
             SET published_at = now(), published_stream_seqs = ARRAY[$2]::bigint[] WHERE id = $1",
        )
        .bind(id)
        .bind(sequence)
        .execute(pool)
        .await
        .expect("the row is marked published");
        published.push((id, sequence));
    }
    published
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

/// `(owner, sequences)` of every command, in outbox order.
async fn requested(pool: &sqlx::PgPool) -> Vec<(String, Vec<u64>)> {
    commands(pool)
        .await
        .into_iter()
        .map(|command| {
            (
                command.payload.owner.to_string(),
                command.payload.stream_sequences,
            )
        })
        .collect()
}

#[tokio::test]
async fn deleting_an_archive_asks_edge_to_erase_the_facts_it_removed() {
    let fixture = Fixture::new().await;
    let first = fixture.import(Some(USER_A), &[("conv-a", "v1")]).await;
    fixture.import(Some(USER_A), &[("conv-a", "v2")]).await;
    let published = publish_as_the_pump_does(fixture.database.pool()).await;
    assert_eq!(
        published.len(),
        4,
        "two heads, an added and an updated fact"
    );
    let request = {
        let service = fixture.service();
        let request = Uuid::now_v7();
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
        service
            .execute(request)
            .await
            .expect("the deletion completes");
        request
    };

    let commands = commands(fixture.database.pool()).await;

    assert_eq!(commands.len(), 1, "one command for the one owner");
    let command = &commands[0];
    assert_eq!(
        command.payload.stream_sequences,
        vec![1, 2],
        "the first export's head and its conversation fact; the retained export's facts stay"
    );
    assert_eq!(command.payload.owner.to_string(), owner(USER_A));
    assert_eq!(
        command.envelope.tenant_id.map(|tenant| tenant.to_string()),
        Some(owner(USER_A)),
        "the envelope tenant equals the payload owner"
    );
    assert_eq!(
        command.envelope.aggregate_id.to_wire(),
        format!("privacy_request:{request}")
    );
    assert_eq!(command.payload.request_ref, command.envelope.aggregate_id);
    assert_eq!(command.envelope.producer.as_str(), "ratatoskr-chatgpt");
    assert_eq!(command.envelope.command_type.to_wire(), COMMAND_TYPE);
    fixture.database.discard().await;
}

#[tokio::test]
async fn a_tenant_deletion_covers_every_fact_by_owner_and_never_a_tombstone_or_a_report() {
    let fixture = Fixture::new().await;
    let first = fixture.import(Some(USER_A), &[("conv-a", "v1")]).await;
    fixture.import(Some(USER_B), &[("conv-a", "v2")]).await;
    let published = publish_as_the_pump_does(fixture.database.pool()).await;
    assert_eq!(published.len(), 4, "the four facts of the two imports");
    // An operation report and an acknowledged tombstone, both published with sequences of their
    // own, are removed by the deletion but are never listed for erasure.
    sqlx::query(
        "INSERT INTO chatgpt_archive.outbox_events
         (event_type, aggregate_id, tenant_id, payload, published_at, published_stream_seqs)
         VALUES ('platform.operation.reported.v1', $1, $2, $3, now(), ARRAY[900]::bigint[])",
    )
    .bind(Uuid::now_v7())
    .bind(first.account)
    .bind(serde_json::json!({"tenant_id": owner(USER_A)}))
    .execute(fixture.database.pool())
    .await
    .expect("the report is inserted");
    sqlx::query(
        "INSERT INTO chatgpt_archive.outbox_events
         (event_type, aggregate_id, tenant_id, payload, published_at, published_stream_seqs,
          acknowledged_at, ack_outcome)
         VALUES ('ai_archive.subject.tombstoned.v1', $1, $2, $3, now(), ARRAY[901]::bigint[], now(),
                 'applied')",
    )
    .bind(Uuid::now_v7())
    .bind(first.account)
    .bind(serde_json::json!({"tenant_id": owner(USER_A), "event_id": Uuid::now_v7().to_string()}))
    .execute(fixture.database.pool())
    .await
    .expect("the tombstone is inserted");

    fixture
        .delete(first.account, PrivacyDeletionScope::Tenant)
        .await;

    assert_eq!(
        requested(fixture.database.pool()).await,
        vec![(owner(USER_A), vec![1, 2]), (owner(USER_B), vec![3, 4]),],
        "one command per owner, all their facts including the superseded conversation.updated, \
         and not the report or the tombstone"
    );
    fixture.database.discard().await;
}

#[tokio::test]
async fn an_unpublished_fact_adds_no_sequence_and_no_command() {
    let fixture = Fixture::new().await;
    let received = fixture
        .import(Some(USER_A), &[("conv-a", "a"), ("conv-b", "b")])
        .await;
    let scope = PrivacyDeletionScope::Archive {
        ai_archive_id: received.archive,
    };

    // Nothing was published: nothing to erase.
    fixture.delete(received.account, scope).await;
    assert!(
        requested(fixture.database.pool()).await.is_empty(),
        "an unpublished fact needs no command"
    );

    // One of the three facts was published: only its sequence is listed.
    let second = Fixture::new().await;
    let received = second
        .import(Some(USER_A), &[("conv-a", "a"), ("conv-b", "b")])
        .await;
    sqlx::query(
        "UPDATE chatgpt_archive.outbox_events
         SET published_at = now(), published_stream_seqs = ARRAY[7]::bigint[]
         WHERE id = (SELECT min(id) FROM chatgpt_archive.outbox_events
                     WHERE event_type = 'ai_archive.archive.imported.v1')",
    )
    .execute(second.database.pool())
    .await
    .expect("the head is marked published");
    second
        .delete(
            received.account,
            PrivacyDeletionScope::Archive {
                ai_archive_id: received.archive,
            },
        )
        .await;
    assert_eq!(
        requested(second.database.pool()).await,
        vec![(owner(USER_A), vec![7])]
    );
    fixture.database.discard().await;
    second.database.discard().await;
}

#[tokio::test]
async fn more_than_five_hundred_sequences_are_split_into_ascending_commands() {
    let fixture = Fixture::new().await;
    let names: Vec<String> = (0..501).map(|index| format!("conv-{index:03}")).collect();
    let conversations: Vec<(&str, &str)> =
        names.iter().map(|name| (name.as_str(), "text")).collect();
    let received = fixture.import(Some(USER_A), &conversations).await;
    let published = publish_as_the_pump_does(fixture.database.pool()).await;
    assert_eq!(published.len(), 502, "one head and 501 conversations");

    fixture
        .delete(
            received.account,
            PrivacyDeletionScope::Archive {
                ai_archive_id: received.archive,
            },
        )
        .await;

    let requested = requested(fixture.database.pool()).await;
    assert_eq!(requested.len(), 2, "502 sequences need two commands");
    assert_eq!(requested[0].1, (1..=500).collect::<Vec<u64>>());
    assert_eq!(requested[1].1, vec![501, 502]);
    assert!(requested.iter().all(|(who, _)| who == &owner(USER_A)));
    fixture.database.discard().await;
}

#[tokio::test]
async fn the_survivor_path_erases_the_dropped_facts() {
    let fixture = Fixture::new().await;
    let first = fixture.import(Some(USER_A), &[("conv-a", "a")]).await;
    fixture
        .import(Some(USER_A), &[("conv-a", "a"), ("conv-b", "b")])
        .await;
    publish_as_the_pump_does(fixture.database.pool()).await;

    fixture
        .delete(
            first.account,
            PrivacyDeletionScope::Archive {
                ai_archive_id: first.archive,
            },
        )
        .await;

    assert_eq!(
        requested(fixture.database.pool()).await,
        vec![(owner(USER_A), vec![1, 2])],
        "the dropped head and the conversation fact the survivor was announced under; the \
         retained export's facts stay"
    );
    let reannounced: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chatgpt_archive.outbox_events
         WHERE event_type = 'ai_archive.conversation.updated.v1'",
    )
    .fetch_one(fixture.database.pool())
    .await
    .expect("the outbox is readable");
    assert_eq!(reannounced, 1, "the survivor is announced again");
    fixture.database.discard().await;
}

#[tokio::test]
async fn a_later_tenant_deletion_keeps_an_unpublished_erasure_command() {
    let fixture = Fixture::new().await;
    let first = fixture.import(Some(USER_A), &[("conv-a", "a")]).await;
    fixture.import(Some(USER_A), &[("conv-b", "b")]).await;
    publish_as_the_pump_does(fixture.database.pool()).await;
    fixture
        .delete(
            first.account,
            PrivacyDeletionScope::Archive {
                ai_archive_id: first.archive,
            },
        )
        .await;
    assert_eq!(requested(fixture.database.pool()).await.len(), 1);

    fixture
        .delete(first.account, PrivacyDeletionScope::Tenant)
        .await;

    let kept = requested(fixture.database.pool()).await;
    assert!(
        kept.iter()
            .any(|(who, sequences)| who == &owner(USER_A) && sequences == &vec![1, 2]),
        "the earlier unpublished command survives the tenant deletion: {kept:?}"
    );
    fixture.database.discard().await;
}

/// Edge as the events and commands streams the producers publish into.
async fn streams(broker: &PrivateBroker) -> (jetstream::stream::Stream, jetstream::stream::Stream) {
    let client = async_nats::connect(broker.url())
        .await
        .expect("the private broker accepts connections");
    let context = jetstream::new(client);
    let events = context
        .create_stream(jetstream::stream::Config {
            name: "ratatoskr_events".to_owned(),
            subjects: vec!["evt.>".to_owned()],
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
    (events, commands)
}

#[tokio::test]
async fn the_pump_publishes_the_command_and_a_platform_handler_erases_only_the_named_messages() {
    let broker = PrivateBroker::plain();
    let (events, commands_stream) = streams(&broker).await;
    let fixture = Fixture::new().await;
    let first = fixture
        .import(Some(USER_A), &[("conv-a", "secret of a")])
        .await;
    fixture
        .import(Some(USER_B), &[("conv-x", "secret of b")])
        .await;
    let seed = seed_file();
    let pump = ArchiveEventOutbox::new(fixture.database.pool().clone());
    pump.publish_pending_once(&broker.url(), seed.path())
        .await
        .expect("the pump publishes the facts");
    let recorded: Vec<(String, i64)> = sqlx::query_as(
        "SELECT payload ->> 'tenant_id', published_stream_seqs[1]
         FROM chatgpt_archive.outbox_events ORDER BY id",
    )
    .fetch_all(fixture.database.pool())
    .await
    .expect("the outbox is readable");
    assert_eq!(recorded.len(), 4);

    fixture
        .delete(
            first.account,
            PrivacyDeletionScope::Archive {
                ai_archive_id: first.archive,
            },
        )
        .await;
    pump.publish_pending_once(&broker.url(), seed.path())
        .await
        .expect("the pump publishes the command");

    let message = commands_stream
        .get_last_raw_message_by_subject("cmd.platform.stream_erasure.requested.v1")
        .await
        .expect("the command is on its subject");
    let envelope = CommandEnvelope::from_json(&message.payload).expect("a command envelope");
    assert_eq!(
        message.headers.get("Nats-Msg-Id").map(ToString::to_string),
        Some(envelope.command_id.to_string()),
        "the command is named by its command id"
    );
    let request = envelope
        .payload_as::<PlatformStreamErasureRequested>()
        .expect("the erasure request");
    // The fake of the Platform handler: erase what the command names, nothing else.
    for sequence in &request.stream_sequences {
        events
            .delete_message(*sequence)
            .await
            .expect("the named message is erased");
    }
    let sequence_of_a: Vec<u64> = recorded
        .iter()
        .filter(|(who, _)| who == &owner(USER_A))
        .map(|(_, sequence)| u64::try_from(*sequence).expect("a positive sequence"))
        .collect();
    assert_eq!(request.stream_sequences, sequence_of_a);
    for sequence in &sequence_of_a {
        assert!(
            events.get_raw_message(*sequence).await.is_err(),
            "the fact message {sequence} is gone from the stream"
        );
    }
    for (who, sequence) in recorded.iter().filter(|(who, _)| who == &owner(USER_B)) {
        assert!(
            events
                .get_raw_message(u64::try_from(*sequence).expect("a positive sequence"))
                .await
                .is_ok(),
            "a message of {who} is untouched"
        );
    }
    fixture.database.discard().await;
}
