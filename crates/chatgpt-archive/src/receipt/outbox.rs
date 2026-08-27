//! Delivery of terminal receipt reports from the durable local outbox.

use sqlx::Row as _;

/// Subject Platform consumes to project producer operation facts.
const OPERATION_REPORTED_SUBJECT: &str = "evt.platform.operation.reported.v1";
const BATCH_SIZE: i64 = 32;

/// The persistent queue that delivers terminal receipt reports to `JetStream`.
#[derive(Debug, Clone)]
pub struct OperationReportOutbox {
    pool: sqlx::PgPool,
}

impl OperationReportOutbox {
    /// Uses the established service database pool; it never creates a pool per pass.
    #[must_use]
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Publishes one bounded batch and marks only broker-acknowledged rows.
    ///
    /// Each row id is the `JetStream` message id. A crash after acknowledgement
    /// but before the SQL update can redeliver, but `JetStream` collapses it and
    /// Platform's operation projection remains idempotent.
    ///
    /// # Errors
    ///
    /// Returns [`OutboxError`] when the broker does not acknowledge a message
    /// or the durable queue cannot be read or marked as published.
    pub async fn publish_pending_once(&self, endpoint: &str) -> Result<usize, OutboxError> {
        let client = async_nats::connect(endpoint)
            .await
            .map_err(OutboxError::broker)?;
        let jetstream = async_nats::jetstream::new(client);
        let rows = sqlx::query(
            "SELECT id, payload FROM chatgpt_archive.outbox_events \
             WHERE event_type = 'platform.operation.reported.v1' AND published_at IS NULL \
             ORDER BY id LIMIT $1",
        )
        .bind(BATCH_SIZE)
        .fetch_all(&self.pool)
        .await
        .map_err(OutboxError::Database)?;

        let mut published = 0;
        for row in rows {
            let id: i64 = row.try_get("id").map_err(OutboxError::Database)?;
            let payload: serde_json::Value =
                row.try_get("payload").map_err(OutboxError::Database)?;
            let body = serde_json::to_vec(&payload).map_err(OutboxError::Encode)?;
            let mut headers = async_nats::HeaderMap::new();
            headers.insert("Nats-Msg-Id", id.to_string());
            let acknowledgement = jetstream
                .publish_with_headers(OPERATION_REPORTED_SUBJECT, headers, body.into())
                .await
                .map_err(OutboxError::broker)?;
            acknowledgement.await.map_err(OutboxError::broker)?;
            sqlx::query(
                "UPDATE chatgpt_archive.outbox_events SET published_at = now() \
                 WHERE id = $1 AND published_at IS NULL",
            )
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(OutboxError::Database)?;
            published += 1;
        }
        Ok(published)
    }
}

/// Why an outbox pass could not progress; callers keep the durable row pending.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OutboxError {
    /// The broker was unreachable or did not acknowledge the message.
    #[error("the operation report broker was unavailable")]
    Broker(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The local queue could not be read or acknowledged as published.
    #[error("the operation report outbox database operation failed")]
    Database(#[source] sqlx::Error),
    /// A locally stored JSON payload could not be encoded for the bus.
    #[error("the operation report payload could not be encoded")]
    Encode(#[source] serde_json::Error),
}

impl OutboxError {
    fn broker(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Broker(Box::new(error))
    }
}
