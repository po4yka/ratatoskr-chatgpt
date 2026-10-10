//! Published normalized-event conformance and linkage tests.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "synthetic contract fixtures"
)]

mod common;

use ratatoskr_ai_archive_contracts::{
    AiArchiveImport, AiArchiveProvenance, AiArchiveTombstone, AiConversation, AiConversationAdded,
    AiProject, AiProjectAdded,
};
use ratatoskr_chatgpt_archive::NormalizedArchiveEvent;
use ratatoskr_identifiers::{ContentDigest, DigestAlgorithm, DigestHex, Extensions};

const PROVENANCE: &str = r#"{
  "ai_archive_id":"018f0000-0000-7000-8000-000000000402",
  "provider":"chatgpt", "owner":"user:018f0000-0000-7000-8000-000000000005",
  "source_export":{"owner_service":"ratatoskr-chatgpt","digest":{"algorithm":"sha256","hex":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"media_type":"application/json","length_bytes":512},
  "imported_at":"2026-08-17T10:00:00Z", "parser_name":"chatgpt_export", "parser_version":"2026.08.1"
}"#;

const IMPORT: &str = r#"{
  "ai_archive_id":"018f0000-0000-7000-8000-000000000402",
  "provider":"chatgpt", "owner":"user:018f0000-0000-7000-8000-000000000005",
  "source_export":{"owner_service":"ratatoskr-chatgpt","digest":{"algorithm":"sha256","hex":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"media_type":"application/json","length_bytes":512},
  "imported_at":"2026-08-17T10:00:00Z", "parser_name":"chatgpt_export", "parser_version":"2026.08.1",
  "completeness_report":{"completeness":"complete","conversation_count":1,"message_count":1,"asset_count":0,"gap_count":0}
}"#;

const CONVERSATION: &str = r#"{
  "ai_conversation_id":"018f0000-0000-7000-8000-000000000403",
  "provider":"chatgpt", "owner":"user:018f0000-0000-7000-8000-000000000005",
  "messages":[{"external_message_id":"msg-0001","author_role":"user","parts":[{"part_kind":"text","text":"Evidence."}],"parser_name":"chatgpt_export","parser_version":"2026.08.1"}],
  "content_digest":{"algorithm":"sha256","hex":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},
  "parser_name":"chatgpt_export","parser_version":"2026.08.1"
}"#;

const PROJECT: &str = r#"{
  "ai_project_id":"018f0000-0000-7000-8000-000000000404", "provider":"chatgpt",
  "title":"Rust notes", "parser_name":"chatgpt_export", "parser_version":"2026.08.1"
}"#;

#[test]
fn import_event_round_trips_the_published_contract_fixture()
-> Result<(), Box<dyn std::error::Error>> {
    let payload: AiArchiveImport = serde_json::from_str(IMPORT)?;
    let event = NormalizedArchiveEvent::archive_imported(&payload)?;
    assert_eq!(event.event_type, "ai_archive.archive.imported.v1");
    let round_trip: AiArchiveImport = event.envelope.payload_as()?;
    round_trip.validate()?;
    assert_eq!(
        round_trip.ai_archive_id.to_string(),
        "018f0000-0000-7000-8000-000000000402"
    );
    Ok(())
}

#[test]
fn conversation_event_round_trips_the_published_contract_fixture()
-> Result<(), Box<dyn std::error::Error>> {
    // The contract verifies a conversation fact's digest, and a producer sets it only
    // through the contract function, so the fixture's placeholder digest is recomputed.
    let mut conversation = serde_json::from_str::<AiConversation>(CONVERSATION)?;
    conversation.content_digest = AiConversation::compute_content_digest(&conversation.messages)?;
    let payload = AiConversationAdded {
        import_provenance: serde_json::from_str::<AiArchiveProvenance>(PROVENANCE)?,
        conversation,
        extensions: Extensions::new(),
    };
    let event = NormalizedArchiveEvent::conversation_added(&payload)?;
    assert_eq!(event.event_type, "ai_archive.conversation.added.v1");
    let round_trip: AiConversationAdded = event.envelope.payload_as()?;
    round_trip.validate()?;
    assert_eq!(
        round_trip.import_provenance.ai_archive_id.to_string(),
        "018f0000-0000-7000-8000-000000000402"
    );
    Ok(())
}

#[test]
fn project_event_round_trips_import_provenance_and_content_digest()
-> Result<(), Box<dyn std::error::Error>> {
    let digest = ContentDigest {
        algorithm: DigestAlgorithm::Sha256,
        hex: DigestHex::parse("1111111111111111111111111111111111111111111111111111111111111111")?,
    };
    let payload = AiProjectAdded {
        import_provenance: serde_json::from_str(PROVENANCE)?,
        project: serde_json::from_str::<AiProject>(PROJECT)?,
        content_digest: digest,
        extensions: Extensions::new(),
    };
    let event = NormalizedArchiveEvent::project_added(&payload)?;
    assert_eq!(event.event_type, "ai_archive.project.added.v1");
    let round_trip: AiProjectAdded = event.envelope.payload_as()?;
    round_trip.validate()?;
    Ok(())
}

#[test]
fn tombstone_event_round_trips_authoritative_deletion_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let tombstone: AiArchiveTombstone = serde_json::from_str(
        r#"{
      "ai_archive_id":"018f0000-0000-7000-8000-000000000402", "provider":"chatgpt",
      "owner":"user:018f0000-0000-7000-8000-000000000005", "subject":{"subject_kind":"archive"},
      "reason":"provider_deletion_event", "evidence_ref":{"owner_service":"ratatoskr-chatgpt","digest":{"algorithm":"sha256","hex":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"media_type":"application/json","length_bytes":512}, "observed_at":"2026-08-27T06:00:00Z"
    }"#,
    )?;
    let event = NormalizedArchiveEvent::tombstoned(&tombstone)?;
    assert_eq!(event.event_type, "ai_archive.subject.tombstoned.v1");
    let round_trip: AiArchiveTombstone = event.envelope.payload_as()?;
    assert_eq!(
        round_trip.subject,
        ratatoskr_ai_archive_contracts::AiArchiveTombstoneSubject::Archive
    );
    Ok(())
}

#[test]
fn user_requested_deletion_event_round_trips() {
    let parsed = serde_json::from_str::<AiArchiveTombstone>(
        r#"{
      "ai_archive_id":"018f0000-0000-7000-8000-000000000402", "provider":"chatgpt",
      "owner":"user:018f0000-0000-7000-8000-000000000005", "subject":{"subject_kind":"conversation","ai_conversation_id":"018f0000-0000-7000-8000-000000000403"},
      "reason":"user_requested", "evidence_ref":{"owner_service":"ratatoskr-chatgpt","digest":{"algorithm":"sha256","hex":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"media_type":"application/json","length_bytes":512}, "observed_at":"2026-08-27T06:00:00Z"
    }"#,
    );
    assert!(
        parsed.is_ok(),
        "the published deletion reason must deserialize: {parsed:?}"
    );

    let event = NormalizedArchiveEvent::tombstoned(&parsed.expect("asserted successful parse"))
        .expect("a valid tombstone must encode");
    assert_eq!(event.event_type, "ai_archive.subject.tombstoned.v1");
    let round_trip = event.envelope.payload_as::<AiArchiveTombstone>();
    assert!(round_trip.is_ok(), "encoded payload must round-trip");
}

// ---- delivery through a real JetStream (XR-021 CONTRACTS.md sections S02 and S07) ----

const OWNER: &str = "user:018f0000-0000-7000-8000-000000000005";

fn tombstone(subject: &str) -> AiArchiveTombstone {
    serde_json::from_value(serde_json::json!({
        "ai_archive_id": "018f0000-0000-7000-8000-000000000402",
        "provider": "chatgpt",
        "owner": OWNER,
        "subject": serde_json::from_str::<serde_json::Value>(subject).expect("literal subject"),
        "reason": "user_requested",
        "evidence_ref": {
            "owner_service": "ratatoskr-chatgpt",
            "digest": {"algorithm": "sha256", "hex": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},
            "media_type": "application/json",
            "length_bytes": 512
        },
        "observed_at": "2026-08-27T06:00:00Z"
    }))
    .expect("a literal tombstone satisfies the contract")
}

/// A throwaway service identity. The test broker does not authenticate, but the
/// pump connects exactly as it does in production, with an nkey seed read from a file.
fn seed_file() -> tempfile::NamedTempFile {
    use std::io::Write as _;
    let seed = nkeys::KeyPair::new_user()
        .seed()
        .expect("a generated key pair has a seed");
    let mut file = tempfile::NamedTempFile::new().expect("a seed file");
    file.write_all(seed.as_bytes())
        .expect("the seed is written");
    file
}

/// One test at a time owns the shared stream.
async fn stream_guard() -> tokio::sync::MutexGuard<'static, ()> {
    static GUARD: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    GUARD
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

const STREAM: &str = "xr021_chatgpt_events";

async fn fresh_events_stream(
    jetstream: &async_nats::jetstream::Context,
) -> async_nats::jetstream::stream::Stream {
    stream_over(jetstream, vec!["evt.>".to_owned()]).await
}

async fn stream_over(
    jetstream: &async_nats::jetstream::Context,
    subjects: Vec<String>,
) -> async_nats::jetstream::stream::Stream {
    let _ = jetstream.delete_stream(STREAM).await;
    jetstream
        .create_stream(async_nats::jetstream::stream::Config {
            name: STREAM.to_owned(),
            subjects,
            ..Default::default()
        })
        .await
        .expect("the test stream is created")
}

async fn enqueue(pool: &sqlx::PgPool, event: &NormalizedArchiveEvent) {
    let mut transaction = pool.begin().await.expect("a transaction");
    ratatoskr_chatgpt_archive::Database::enqueue_normalized_event(
        &mut transaction,
        event,
        &ratatoskr_chatgpt_archive::OutboxPlacement::default(),
    )
    .await
    .expect("the event is enqueued");
    transaction.commit().await.expect("the commit succeeds");
}

fn pump(pool: &sqlx::PgPool) -> ratatoskr_chatgpt_archive::receipt::ArchiveEventOutbox {
    ratatoskr_chatgpt_archive::receipt::ArchiveEventOutbox::new(pool.clone())
}

#[tokio::test]
async fn tombstone_is_published_as_a_contract_envelope() {
    let _guard = stream_guard().await;
    let database = common::DisposableDatabase::create().await;
    let client = async_nats::connect(common::nats_url())
        .await
        .expect("the test broker accepts connections");
    let jetstream = async_nats::jetstream::new(client);
    let mut stream = fresh_events_stream(&jetstream).await;
    let event = NormalizedArchiveEvent::tombstoned(&tombstone(r#"{"subject_kind":"archive"}"#))
        .expect("a valid tombstone encodes");
    enqueue(database.pool(), &event).await;
    let seed = seed_file();

    let published = pump(database.pool())
        .publish_pending_once(&common::nats_url(), seed.path())
        .await
        .expect("the pump publishes");

    assert_eq!(published, 1, "the unpublished tombstone row is delivered");
    let message = stream
        .get_last_raw_message_by_subject("evt.ai_archive.subject.tombstoned.v1")
        .await
        .expect("the broker holds the message on the contract subject");
    let envelope = ratatoskr_event_envelope::EventEnvelope::from_json(&message.payload)
        .expect("the broker message is a complete event envelope");
    assert_eq!(envelope.producer.as_str(), "ratatoskr-chatgpt");
    let tombstone: AiArchiveTombstone = envelope
        .payload_as()
        .expect("the envelope payload is the tombstone");
    assert_eq!(tombstone.owner.to_string(), OWNER);
    assert_eq!(stream.info().await.expect("stream info").state.messages, 1);
    let _ = jetstream.delete_stream(STREAM).await;
    database.discard().await;
}

#[tokio::test]
async fn pending_rows_include_unpublished_tombstones_in_id_order() {
    let _guard = stream_guard().await;
    let database = common::DisposableDatabase::create().await;
    let client = async_nats::connect(common::nats_url())
        .await
        .expect("the test broker accepts connections");
    let jetstream = async_nats::jetstream::new(client);
    let stream = fresh_events_stream(&jetstream).await;
    for subject in [
        r#"{"subject_kind":"archive"}"#,
        r#"{"subject_kind":"project","ai_project_id":"018f0000-0000-7000-8000-000000000404"}"#,
        r#"{"subject_kind":"conversation","ai_conversation_id":"018f0000-0000-7000-8000-000000000403"}"#,
    ] {
        let event = NormalizedArchiveEvent::tombstoned(&tombstone(subject))
            .expect("a valid tombstone encodes");
        enqueue(database.pool(), &event).await;
    }
    let seed = seed_file();

    let published = pump(database.pool())
        .publish_pending_once(&common::nats_url(), seed.path())
        .await
        .expect("the pump publishes");

    assert_eq!(published, 3);
    let mut ids = Vec::new();
    for sequence in 1..=3 {
        let message = stream
            .get_raw_message(sequence)
            .await
            .expect("the stream holds the message");
        let id = message
            .headers
            .get("Nats-Msg-Id")
            .map(ToString::to_string)
            .expect("every message carries its row id");
        ids.push(id);
    }
    assert_eq!(ids, ["1", "2", "3"], "rows go out in id order");
    let _ = jetstream.delete_stream(STREAM).await;
    database.discard().await;
}

#[tokio::test]
async fn an_event_type_outside_the_closed_list_is_a_hard_error() {
    let _guard = stream_guard().await;
    let database = common::DisposableDatabase::create().await;
    let client = async_nats::connect(common::nats_url())
        .await
        .expect("the test broker accepts connections");
    let jetstream = async_nats::jetstream::new(client);
    fresh_events_stream(&jetstream).await;
    // The schema forbids such a row. Remove that guard to prove the publisher does
    // not rely on it.
    sqlx::query(
        "ALTER TABLE chatgpt_archive.outbox_events DROP CONSTRAINT IF EXISTS outbox_event_type_closed",
    )
    .execute(database.pool())
    .await
    .expect("the constraint can be dropped");
    sqlx::query(
        "INSERT INTO chatgpt_archive.outbox_events (event_type, aggregate_id, payload)
         VALUES ('social.source.captured.v1', $1, '{}')",
    )
    .bind(uuid::Uuid::now_v7())
    .execute(database.pool())
    .await
    .expect("the row is inserted");
    let seed = seed_file();

    let outcome = pump(database.pool())
        .publish_pending_once(&common::nats_url(), seed.path())
        .await;

    assert!(
        outcome.is_err(),
        "an unknown type is never silently skipped"
    );
    let unpublished: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chatgpt_archive.outbox_events WHERE published_at IS NULL",
    )
    .fetch_one(database.pool())
    .await
    .expect("the outbox is readable");
    assert_eq!(unpublished, 1, "the row stays unpublished");
    let _ = jetstream.delete_stream(STREAM).await;
    database.discard().await;
}

#[tokio::test]
async fn the_schema_rejects_an_event_type_outside_the_closed_list() {
    let database = common::DisposableDatabase::create().await;

    let inserted = sqlx::query(
        "INSERT INTO chatgpt_archive.outbox_events (event_type, aggregate_id, payload)
         VALUES ('social.source.captured.v1', $1, '{}')",
    )
    .bind(uuid::Uuid::now_v7())
    .execute(database.pool())
    .await;

    assert!(
        inserted.is_err(),
        "a row the publisher cannot map must be impossible to insert"
    );
    database.discard().await;
}

#[tokio::test]
async fn a_refused_row_backs_off_without_starving_the_rows_behind_it() {
    let _guard = stream_guard().await;
    let database = common::DisposableDatabase::create().await;
    let client = async_nats::connect(common::nats_url())
        .await
        .expect("the test broker accepts connections");
    let jetstream = async_nats::jetstream::new(client);
    // Only tombstones have a stream, so a publish of the import head is refused as the
    // broker refuses a subject the identity may not publish: no acknowledgement.
    let mut stream = stream_over(
        &jetstream,
        vec!["evt.ai_archive.subject.tombstoned.v1".to_owned()],
    )
    .await;
    let head: AiArchiveImport = serde_json::from_str(IMPORT).expect("the head fixture parses");
    enqueue(
        database.pool(),
        &NormalizedArchiveEvent::archive_imported(&head).expect("the head encodes"),
    )
    .await;
    enqueue(
        database.pool(),
        &NormalizedArchiveEvent::tombstoned(&tombstone(r#"{"subject_kind":"archive"}"#))
            .expect("a valid tombstone encodes"),
    )
    .await;
    let seed = seed_file();

    let outcome = pump(database.pool())
        .publish_pending_once(&common::nats_url(), seed.path())
        .await;

    assert!(
        outcome.is_err(),
        "the refused row is reported, never swallowed"
    );
    assert_eq!(
        stream.info().await.expect("stream info").state.messages,
        1,
        "the row behind the refused one was still delivered"
    );
    let rows: Vec<(i64, bool, i32, Option<String>, bool)> = sqlx::query_as(
        "SELECT id, published_at IS NOT NULL, attempt_count, last_error,
                next_attempt_at > now()
         FROM chatgpt_archive.outbox_events ORDER BY id",
    )
    .fetch_all(database.pool())
    .await
    .expect("the outbox is readable");
    assert_eq!(
        rows,
        [
            (1, false, 1, Some("not_acknowledged".to_owned()), true),
            (2, true, 0, None, false),
        ],
        "the refused row records one attempt and waits; the delivered row is marked"
    );
    let _ = jetstream.delete_stream(STREAM).await;
    database.discard().await;
}
