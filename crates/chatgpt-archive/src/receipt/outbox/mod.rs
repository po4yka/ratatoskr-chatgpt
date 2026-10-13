//! Delivery of archive events from the durable local outbox.
//!
//! One pump serves every event type this service produces: the Platform operation
//! report and the eight `ai_archive.*` facts. Each stored row already holds the
//! complete envelope, so the pump maps `event_type` to its subject through one closed
//! table, publishes the stored document unchanged, and marks the row only after the
//! broker acknowledgement resolves (XR-021 CONTRACTS.md sections S02 and S07).

mod error;
mod marks;
mod subject;

pub use error::PublishError;
pub(crate) use subject::subject_for_event_type;

use sqlx::Row as _;

use crate::deletion_ack::{TOMBSTONE_EVENT_TYPE, TombstoneResend};

use subject::{publish_row, stored_envelope};

/// Room kept below the broker's `max_payload` for headers and framing (XR-021 CONTRACTS.md R2-04
/// rule 2).
const PAYLOAD_HEADROOM_BYTES: usize = 1024;
/// The safe failure class stored for a row that can never be delivered.
const CLASS_PAYLOAD_TOO_LARGE: &str = "payload_too_large";
/// The reply prefix of this identity. The deployed ACL lets the service subscribe only to
/// `_INBOX_chatgpt.>`, so every connection it opens must set it (XR-021 CONTRACTS.md R3-01);
/// without it a publish acknowledgement is never delivered and the call times out.
pub(crate) const INBOX_PREFIX: &str = "_INBOX_chatgpt";
const BATCH_SIZE: i64 = 32;
const MAX_BACKOFF_SECONDS: i32 = 300;

/// The options of every broker connection this service opens: the nkey identity and the inbox
/// prefix its ACL stanza allows.
#[must_use]
pub(crate) fn connect_options(nkey_seed: &str) -> async_nats::ConnectOptions {
    async_nats::ConnectOptions::with_nkey(nkey_seed.trim().to_owned())
        .custom_inbox_prefix(INBOX_PREFIX)
}

/// The persistent queue that delivers archive events to `JetStream`.
///
/// Clones share one broker connection, created the first time a pass has something to publish.
#[derive(Debug, Clone)]
pub struct ArchiveEventOutbox {
    pool: sqlx::PgPool,
    resend: TombstoneResend,
    connection: std::sync::Arc<tokio::sync::Mutex<Option<Connection>>>,
}

/// The broker connection of the pump and whether its client has been closed.
#[derive(Debug, Clone)]
struct Connection {
    client: async_nats::Client,
    closed: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Connection {
    fn is_closed(&self) -> bool {
        self.closed.load(std::sync::atomic::Ordering::Acquire)
    }

    fn mark_closed(&self) {
        self.closed
            .store(true, std::sync::atomic::Ordering::Release);
    }
}

impl ArchiveEventOutbox {
    /// Uses the established service database pool; it never creates a pool per pass.
    #[must_use]
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            pool,
            resend: TombstoneResend::default(),
            connection: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    /// Sets how long a published tombstone waits for Knowledge's acknowledgement before it is
    /// published again (XR-021 CONTRACTS.md R5-11).
    #[must_use]
    pub fn with_tombstone_resend(mut self, resend: TombstoneResend) -> Self {
        self.resend = resend;
        self
    }

    /// Publishes one bounded batch through the pump's broker connection.
    ///
    /// A pass with no pending row returns before touching the network. The first pass
    /// that has a row connects with the service identity; later passes reuse that
    /// connection. async-nats reconnects by itself while the connection is merely
    /// disconnected, and the pump opens a new one only after the client reports it
    /// closed (XR-021 CONTRACTS.md R3-07 rule 1).
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
        crate::deletion_ack::refresh_gauges(&self.pool)
            .await
            .map_err(PublishError::Database)?;
        let rows = self.pending_rows().await?;
        if rows.is_empty() {
            return Ok(0);
        }
        let connection = self.connection(endpoint, nkey_seed_path).await?;
        self.publish_rows(&connection, rows).await
    }

    /// Drains the pump's connection so queued publishes are flushed before the service
    /// exits. The next pass, if there is one, opens a new connection.
    pub async fn close(&self) {
        if let Some(connection) = self.connection.lock().await.as_ref() {
            connection.mark_closed();
            let _ = connection.client.drain().await;
        }
    }

    /// The live connection, or a new one when none exists yet or the client was closed.
    async fn connection(
        &self,
        endpoint: &str,
        nkey_seed_path: &std::path::Path,
    ) -> Result<Connection, PublishError> {
        let mut slot = self.connection.lock().await;
        if let Some(connection) = slot.as_ref().filter(|connection| !connection.is_closed()) {
            return Ok(connection.clone());
        }
        let seed = tokio::fs::read_to_string(nkey_seed_path)
            .await
            .map_err(PublishError::broker)?;
        let closed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&closed);
        let client = connect_options(&seed)
            .event_callback(move |event| {
                let flag = std::sync::Arc::clone(&flag);
                async move {
                    if matches!(event, async_nats::Event::Closed) {
                        flag.store(true, std::sync::atomic::Ordering::Release);
                    }
                }
            })
            .connect(endpoint)
            .await
            .map_err(PublishError::broker)?;
        let connection = Connection { client, closed };
        *slot = Some(connection.clone());
        Ok(connection)
    }

    async fn pending_rows(&self) -> Result<Vec<sqlx::postgres::PgRow>, PublishError> {
        // First publications, and the published tombstones whose acknowledgement is overdue
        // (XR-021 CONTRACTS.md R5-11): the second kind is published again, unchanged.
        sqlx::query(
            "SELECT id, event_type, payload, published_at IS NOT NULL AS resend
             FROM chatgpt_archive.outbox_events
             WHERE undeliverable_at IS NULL
               AND ((published_at IS NULL AND next_attempt_at <= now())
                    OR (event_type = $2 AND published_at IS NOT NULL
                        AND acknowledged_at IS NULL AND next_resend_at <= now()))
             ORDER BY id LIMIT $1",
        )
        .bind(BATCH_SIZE)
        .bind(TOMBSTONE_EVENT_TYPE)
        .fetch_all(&self.pool)
        .await
        .map_err(PublishError::Database)
    }

    /// Publishes `rows` in id order and marks only broker-acknowledged rows.
    ///
    /// Each message is named by its envelope `event_id`, so a crash after
    /// acknowledgement but before the SQL update redelivers a message the stream
    /// collapses, and the key space cannot collide with another producer's. A row the
    /// broker did not acknowledge stays unpublished, records a safe failure class and
    /// waits out a bounded backoff, so one refused row never starves the rows behind
    /// it. A transient failure never gives a row up. A row larger than the connected
    /// server accepts can never be delivered: it is marked undeliverable at once,
    /// skipped from then on, counted, and does not fail the pass, so it neither wedges
    /// the pump nor flips readiness.
    ///
    /// # Errors
    ///
    /// Returns [`PublishError::UnknownEventType`] or [`PublishError::InvalidRow`]
    /// for a row this service cannot have produced; both stop the pass without
    /// publishing that row. Returns [`PublishError::Broker`] when any row was not
    /// acknowledged. A `PubAck` timeout is indistinguishable from a permission
    /// denial, so check the NATS server log for a Publish Violation.
    async fn publish_rows(
        &self,
        connection: &Connection,
        rows: Vec<sqlx::postgres::PgRow>,
    ) -> Result<usize, PublishError> {
        let jetstream = async_nats::jetstream::new(connection.client.clone());
        let deliverable_bytes = connection
            .client
            .server_info()
            .max_payload
            .saturating_sub(PAYLOAD_HEADROOM_BYTES);
        let mut published = 0;
        let mut first_failure = None;
        for row in rows {
            let id: i64 = row.try_get("id").map_err(PublishError::Database)?;
            let event_type: String = row.try_get("event_type").map_err(PublishError::Database)?;
            let payload: serde_json::Value =
                row.try_get("payload").map_err(PublishError::Database)?;
            let resend: bool = row.try_get("resend").map_err(PublishError::Database)?;
            let subject =
                subject_for_event_type(&event_type).ok_or(PublishError::UnknownEventType)?;
            let (body, event_id) = stored_envelope(&event_type, &payload)?;
            if body.len() > deliverable_bytes {
                self.mark_undeliverable(id).await?;
                continue;
            }
            match publish_row(&jetstream, subject, &event_id, body).await {
                Ok(sequence) => {
                    if resend {
                        self.mark_resent(id).await?;
                    } else {
                        let mut transaction =
                            self.pool.begin().await.map_err(PublishError::Database)?;
                        marks::mark_published(
                            &mut transaction,
                            id,
                            &[i64::try_from(sequence).unwrap_or(i64::MAX)],
                            self.resend.after,
                        )
                        .await?;
                        transaction.commit().await.map_err(PublishError::Database)?;
                    }
                    published += 1;
                }
                Err(error) => {
                    if error.is_closed_connection() {
                        connection.mark_closed();
                    }
                    if resend {
                        self.record_resend_failure(id).await?;
                    } else {
                        self.record_failure(id).await?;
                    }
                    first_failure.get_or_insert(error);
                }
            }
        }
        first_failure.map_or(Ok(published), Err)
    }
}
