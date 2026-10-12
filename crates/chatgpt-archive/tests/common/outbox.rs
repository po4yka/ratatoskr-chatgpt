//! Helpers for the tests that put archive events in the outbox and run the pump.

use ratatoskr_ai_archive_contracts::AiArchiveTombstone;
use ratatoskr_chatgpt_archive::NormalizedArchiveEvent;

pub(crate) const OWNER: &str = "user:018f0000-0000-7000-8000-000000000005";

/// A valid user-requested tombstone for `subject`, a JSON literal.
pub(crate) fn tombstone(subject: &str) -> AiArchiveTombstone {
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

/// A throwaway service identity. A test broker without authentication ignores it, but the
/// pump connects exactly as it does in production, with an nkey seed read from a file.
pub(crate) fn seed_file() -> tempfile::NamedTempFile {
    use std::io::Write as _;
    let seed = nkeys::KeyPair::new_user()
        .seed()
        .expect("a generated key pair has a seed");
    let mut file = tempfile::NamedTempFile::new().expect("a seed file");
    file.write_all(seed.as_bytes())
        .expect("the seed is written");
    file
}

/// Enqueues `event` in the outbox the way the importer does, for a fresh archive account.
pub(crate) async fn enqueue(pool: &sqlx::PgPool, event: &NormalizedArchiveEvent) {
    let account = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chatgpt_archive.accounts (id, external_kind, external_ref)
         VALUES ($1, 'personal', $2)",
    )
    .bind(account)
    .bind(format!("outbox-helper-{account}"))
    .execute(pool)
    .await
    .expect("the account is inserted");
    let mut transaction = pool.begin().await.expect("a transaction");
    ratatoskr_chatgpt_archive::Database::enqueue_normalized_event(
        &mut transaction,
        event,
        &ratatoskr_chatgpt_archive::OutboxPlacement {
            account_id: Some(account),
            ..ratatoskr_chatgpt_archive::OutboxPlacement::default()
        },
    )
    .await
    .expect("the event is enqueued");
    transaction.commit().await.expect("the commit succeeds");
}

/// Enqueues one archive tombstone and returns its event id.
pub(crate) async fn enqueue_tombstone(pool: &sqlx::PgPool) -> String {
    let event = NormalizedArchiveEvent::tombstoned(&tombstone(r#"{"subject_kind":"archive"}"#))
        .expect("a valid tombstone encodes");
    let event_id = event.envelope.event_id.to_string();
    enqueue(pool, &event).await;
    event_id
}
