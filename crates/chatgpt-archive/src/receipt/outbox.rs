//! Delivery of archive events from the durable local outbox.
//!
//! One pump serves every event type this service produces: the Platform operation
//! report and the eight `ai_archive.*` facts. Each stored row already holds the
//! complete envelope, so the pump maps `event_type` to its subject through one closed
//! table, publishes the stored document unchanged, and marks the row only after the
//! broker acknowledgement resolves (XR-021 CONTRACTS.md sections S02 and S07).

use ratatoskr_event_envelope::EventEnvelope;
use sqlx::Row as _;

/// Subject Platform consumes to project producer operation facts.
const OPERATION_REPORTED_SUBJECT: &str = "evt.ai-archive.chatgpt.operation.reported.v1";
const BATCH_SIZE: i64 = 32;
const MAX_BACKOFF_SECONDS: i32 = 300;

/// The subject an outbox `event_type` publishes to, or none when the type is not one
/// this service produces.
///
/// The operation report keeps its per-provider ingress subject; the eight archive
/// facts publish to `evt.` plus their contract type name. The match is closed on
/// purpose: a type outside it is a programming error, never a row to skip.
pub(crate) fn subject_for_event_type(event_type: &str) -> Option<&'static str> {
    Some(match event_type {
        "platform.operation.reported.v1" => OPERATION_REPORTED_SUBJECT,
        "ai_archive.archive.imported.v1" => "evt.ai_archive.archive.imported.v1",
        "ai_archive.conversation.added.v1" => "evt.ai_archive.conversation.added.v1",
        "ai_archive.conversation.updated.v1" => "evt.ai_archive.conversation.updated.v1",
        "ai_archive.project.added.v1" => "evt.ai_archive.project.added.v1",
        "ai_archive.project.updated.v1" => "evt.ai_archive.project.updated.v1",
        "ai_archive.artifact.added.v1" => "evt.ai_archive.artifact.added.v1",
        "ai_archive.artifact.updated.v1" => "evt.ai_archive.artifact.updated.v1",
        "ai_archive.subject.tombstoned.v1" => "evt.ai_archive.subject.tombstoned.v1",
        _ => return None,
    })
}

/// The persistent queue that delivers archive events to `JetStream`.
#[derive(Debug, Clone)]
pub struct ArchiveEventOutbox {
    pool: sqlx::PgPool,
}

impl ArchiveEventOutbox {
    /// Uses the established service database pool; it never creates a pool per pass.
    #[must_use]
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Connects with the service identity and publishes one bounded batch.
    ///
    /// # Errors
    ///
    /// Returns [`PublishError`] when the broker cannot be reached, does not
    /// acknowledge a message, or the durable queue cannot be read or marked.
    pub async fn publish_pending_once(
        &self,
        endpoint: &str,
        nkey_seed_path: &std::path::Path,
    ) -> Result<usize, PublishError> {
        let seed = tokio::fs::read_to_string(nkey_seed_path)
            .await
            .map_err(PublishError::broker)?;
        let options = async_nats::ConnectOptions::with_nkey(seed.trim().to_owned());
        let client = options
            .connect(endpoint)
            .await
            .map_err(PublishError::broker)?;
        self.publish_pending_with(&async_nats::jetstream::new(client))
            .await
    }

    /// Publishes one bounded batch through an established `JetStream` context and
    /// marks only broker-acknowledged rows.
    ///
    /// Rows go out in id order and each row id is the `JetStream` message id, so a
    /// crash after acknowledgement but before the SQL update redelivers a message
    /// the stream collapses. A row the broker did not acknowledge stays unpublished,
    /// records a safe failure class and waits out a bounded backoff, so one refused
    /// row never starves the rows behind it.
    ///
    /// # Errors
    ///
    /// Returns [`PublishError::UnknownEventType`] or [`PublishError::InvalidRow`]
    /// for a row this service cannot have produced; both stop the pass without
    /// publishing that row. Returns [`PublishError::Broker`] when any row was not
    /// acknowledged. A `PubAck` timeout is indistinguishable from a permission
    /// denial, so check the NATS server log for a Publish Violation.
    pub async fn publish_pending_with(
        &self,
        jetstream: &async_nats::jetstream::Context,
    ) -> Result<usize, PublishError> {
        let rows = sqlx::query(
            "SELECT id, event_type, payload FROM chatgpt_archive.outbox_events
             WHERE published_at IS NULL AND next_attempt_at <= now()
             ORDER BY id LIMIT $1",
        )
        .bind(BATCH_SIZE)
        .fetch_all(&self.pool)
        .await
        .map_err(PublishError::Database)?;

        let mut published = 0;
        let mut first_failure = None;
        for row in rows {
            let id: i64 = row.try_get("id").map_err(PublishError::Database)?;
            let event_type: String = row.try_get("event_type").map_err(PublishError::Database)?;
            let payload: serde_json::Value =
                row.try_get("payload").map_err(PublishError::Database)?;
            let subject =
                subject_for_event_type(&event_type).ok_or(PublishError::UnknownEventType)?;
            let body = stored_envelope_bytes(&event_type, &payload)?;
            match publish_row(jetstream, subject, id, body).await {
                Ok(()) => {
                    self.mark_published(id).await?;
                    published += 1;
                }
                Err(error) => {
                    self.record_failure(id).await?;
                    first_failure.get_or_insert(error);
                }
            }
        }
        first_failure.map_or(Ok(published), Err)
    }

    async fn mark_published(&self, id: i64) -> Result<(), PublishError> {
        sqlx::query(
            "UPDATE chatgpt_archive.outbox_events SET published_at = now()
             WHERE id = $1 AND published_at IS NULL",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(PublishError::Database)?;
        Ok(())
    }

    async fn record_failure(&self, id: i64) -> Result<(), PublishError> {
        sqlx::query(
            "UPDATE chatgpt_archive.outbox_events
             SET attempt_count = attempt_count + 1, last_error = 'not_acknowledged',
                 next_attempt_at = now() + make_interval(
                     secs => least(power(2, least(attempt_count, 8))::int, $2))
             WHERE id = $1 AND published_at IS NULL",
        )
        .bind(id)
        .bind(MAX_BACKOFF_SECONDS)
        .execute(&self.pool)
        .await
        .map_err(PublishError::Database)?;
        Ok(())
    }
}

/// The stored document as the bytes to publish, after proving it is the envelope of
/// the row's declared type.
fn stored_envelope_bytes(
    event_type: &str,
    payload: &serde_json::Value,
) -> Result<Vec<u8>, PublishError> {
    let bytes = serde_json::to_vec(payload).map_err(PublishError::Encode)?;
    let envelope = EventEnvelope::from_json(&bytes).map_err(|_| PublishError::InvalidRow)?;
    if envelope.event_type.to_wire() != event_type {
        return Err(PublishError::InvalidRow);
    }
    Ok(bytes)
}

async fn publish_row(
    jetstream: &async_nats::jetstream::Context,
    subject: &'static str,
    id: i64,
    body: Vec<u8>,
) -> Result<(), PublishError> {
    let mut headers = async_nats::HeaderMap::new();
    headers.insert("Nats-Msg-Id", id.to_string());
    let acknowledgement = jetstream
        .publish_with_headers(subject, headers, body.into())
        .await
        .map_err(PublishError::broker)?;
    acknowledgement.await.map_err(PublishError::broker)?;
    Ok(())
}

/// Why an outbox pass could not progress; callers keep the durable row pending.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PublishError {
    /// The broker was unreachable or did not acknowledge the message. A `PubAck`
    /// timeout is indistinguishable from a permission denial: check the NATS server
    /// log for a Publish Violation.
    #[error(
        "the archive event broker did not acknowledge the message (check the NATS server log for a Publish Violation)"
    )]
    Broker(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The local queue could not be read or acknowledged as published.
    #[error("the archive event outbox database operation failed")]
    Database(#[source] sqlx::Error),
    /// A locally stored JSON payload could not be encoded for the bus.
    #[error("the archive event payload could not be encoded")]
    Encode(#[source] serde_json::Error),
    /// A row names an event type this service does not produce. This is a
    /// programming error and never a row to skip.
    #[error("the archive event outbox holds an event type this service does not produce")]
    UnknownEventType,
    /// A row does not hold the complete envelope of its declared event type.
    #[error("the archive event outbox holds a row that is not a complete envelope")]
    InvalidRow,
}

impl PublishError {
    fn broker(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Broker(Box::new(error))
    }

    /// Whether retrying the pass can ever succeed. A row this service cannot have
    /// produced stays wrong until an operator repairs it, so the worker must stop
    /// instead of retrying forever.
    #[must_use]
    pub const fn is_fatal(&self) -> bool {
        matches!(self, Self::UnknownEventType | Self::InvalidRow)
    }
}

#[cfg(test)]
mod tests {
    use super::subject_for_event_type;

    #[test]
    fn operation_reports_keep_their_per_provider_subject() {
        assert_eq!(
            subject_for_event_type("platform.operation.reported.v1"),
            Some("evt.ai-archive.chatgpt.operation.reported.v1")
        );
    }

    #[test]
    fn the_eight_archive_facts_publish_to_evt_plus_their_type() {
        for event_type in [
            "ai_archive.archive.imported.v1",
            "ai_archive.conversation.added.v1",
            "ai_archive.conversation.updated.v1",
            "ai_archive.project.added.v1",
            "ai_archive.project.updated.v1",
            "ai_archive.artifact.added.v1",
            "ai_archive.artifact.updated.v1",
            "ai_archive.subject.tombstoned.v1",
        ] {
            let expected = format!("evt.{event_type}");
            assert_eq!(
                subject_for_event_type(event_type),
                Some(expected.as_str()),
                "{event_type}"
            );
        }
    }

    #[test]
    fn any_other_type_has_no_subject() {
        assert_eq!(subject_for_event_type("social.source.captured.v1"), None);
        assert_eq!(subject_for_event_type(""), None);
    }
}
