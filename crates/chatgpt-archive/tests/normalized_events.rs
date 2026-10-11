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

use common::outbox::{OWNER, enqueue, seed_file, tombstone};

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

// ---- permanent and transient delivery failures (XR-021 CONTRACTS.md R2-04 rule 2) ----

/// A tombstone row whose stored envelope is padded to at least `bytes` bytes. The padding sits in
/// the payload, which the pump relays unchanged, so the row is a valid envelope of its type that
/// no broker with this limit can accept.
async fn enqueue_padded_tombstone(pool: &sqlx::PgPool, bytes: usize) {
    enqueue(
        pool,
        &NormalizedArchiveEvent::tombstoned(&tombstone(r#"{"subject_kind":"archive"}"#))
            .expect("a valid tombstone encodes"),
    )
    .await;
    sqlx::query(
        "UPDATE chatgpt_archive.outbox_events
         SET payload = jsonb_set(payload, '{payload,padding}', to_jsonb(repeat('x', $1)))
         WHERE id = (SELECT max(id) FROM chatgpt_archive.outbox_events)",
    )
    .bind(i32::try_from(bytes).expect("a test padding fits in i32"))
    .execute(pool)
    .await
    .expect("the row is padded");
}

#[tokio::test]
async fn an_undeliverable_row_does_not_wedge_the_pump() {
    let _guard = stream_guard().await;
    let database = common::DisposableDatabase::create().await;
    let client = async_nats::connect(common::nats_url())
        .await
        .expect("the test broker accepts connections");
    let limit = client.server_info().max_payload;
    let jetstream = async_nats::jetstream::new(client);
    let mut stream = fresh_events_stream(&jetstream).await;
    let small = NormalizedArchiveEvent::tombstoned(&tombstone(r#"{"subject_kind":"archive"}"#))
        .expect("a valid tombstone encodes");
    enqueue(database.pool(), &small).await;
    enqueue_padded_tombstone(database.pool(), limit).await;
    enqueue(database.pool(), &small).await;
    let seed = seed_file();

    let pump = pump(database.pool());
    let first = pump
        .publish_pending_once(&common::nats_url(), seed.path())
        .await
        .expect("an undeliverable row is not a failed pass, so readiness stays true");
    assert_eq!(
        first, 2,
        "the rows on both sides of the large one are published"
    );
    let second = pump
        .publish_pending_once(&common::nats_url(), seed.path())
        .await
        .expect("the large row is skipped from now on");
    assert_eq!(second, 0);

    let rows: Vec<(i64, bool, i32, Option<String>, bool)> = sqlx::query_as(
        "SELECT id, published_at IS NOT NULL, attempt_count, last_error,
                undeliverable_at IS NOT NULL
         FROM chatgpt_archive.outbox_events ORDER BY id",
    )
    .fetch_all(database.pool())
    .await
    .expect("the outbox is readable");
    assert_eq!(
        rows,
        [
            (1, true, 0, None, false),
            (2, false, 0, Some("payload_too_large".to_owned()), true),
            (3, true, 0, None, false),
        ],
        "the large row is marked undeliverable with a safe class; no attempt is recorded for a \
         publish that was never made"
    );
    assert_eq!(stream.info().await.expect("stream info").state.messages, 2);
    let _ = jetstream.delete_stream(STREAM).await;
    database.discard().await;
}

#[tokio::test]
async fn transient_refusals_never_make_a_row_undeliverable() {
    let _guard = stream_guard().await;
    let database = common::DisposableDatabase::create().await;
    let client = async_nats::connect(common::nats_url())
        .await
        .expect("the test broker accepts connections");
    let jetstream = async_nats::jetstream::new(client);
    // No stream takes the head's subject, so the broker never acknowledges it: the same shape as
    // an unreachable broker or a permission denial.
    let stream = stream_over(
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
    let seed = seed_file();

    let outcome = pump(database.pool())
        .publish_pending_once(&common::nats_url(), seed.path())
        .await;

    assert!(outcome.is_err(), "a transient refusal fails the pass");
    let undeliverable: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chatgpt_archive.outbox_events WHERE undeliverable_at IS NOT NULL",
    )
    .fetch_one(database.pool())
    .await
    .expect("the outbox is readable");
    assert_eq!(undeliverable, 0, "a refusal is retried, never given up on");
    let (attempts, last_error): (i32, Option<String>) =
        sqlx::query_as("SELECT attempt_count, last_error FROM chatgpt_archive.outbox_events")
            .fetch_one(database.pool())
            .await
            .expect("the row is readable");
    assert_eq!(
        (attempts, last_error.as_deref()),
        (1, Some("not_acknowledged"))
    );
    drop(stream);
    let _ = jetstream.delete_stream(STREAM).await;
    database.discard().await;
}
