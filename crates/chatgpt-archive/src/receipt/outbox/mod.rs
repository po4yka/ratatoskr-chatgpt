//! Delivery of archive events from the durable local outbox.
//!
//! One pump serves every event type this service produces: the Platform operation
//! report and the eight `ai_archive.*` facts. Each stored row already holds the
//! complete envelope, so the pump maps `event_type` to its subject through one closed
//! table, publishes the stored document unchanged, and marks the row only after the
//! broker acknowledgement resolves (XR-021 CONTRACTS.md sections S02 and S07).

mod carry;
mod error;
mod marks;
mod publish;
mod subject;

pub use error::PublishError;

use crate::deletion_ack::{TOMBSTONE_EVENT_TYPE, TombstoneResend};
use crate::receipt::probe::{PublishPause, PublishProbe};

use carry::Carry;

/// Room kept below the broker's `max_payload` for headers and framing (XR-021 CONTRACTS.md R2-04
/// rule 2).
const PAYLOAD_HEADROOM_BYTES: usize = 1024;
/// The safe failure class stored for a row that can never be delivered.
const CLASS_PAYLOAD_TOO_LARGE: &str = "payload_too_large";
/// The reply prefix of this identity. The deployed ACL lets the service subscribe only to
/// `_INBOX_chatgpt.>`, so every connection it opens must set it (XR-021 CONTRACTS.md R3-01);
/// without it a publish acknowledgement is never delivered and the call times out.
pub(crate) const INBOX_PREFIX: &str = "_INBOX_chatgpt";
/// How long the pump waits for the broker, and so the longest it holds a row lock while a first
/// publication is in flight (XR-021 CONTRACTS.md R6-03 rule 2). It is set explicitly on the
/// pump's `JetStream` context instead of inheriting the library default, because a change of that
/// default would silently change how long a deletion can wait behind the pump. Pool consequence:
/// the pump holds one database connection for the whole of that window, so the pool must keep a
/// connection free for the deletion that waits.
pub(crate) const PUBLISH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
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
    probe: Option<std::sync::Arc<PublishProbe>>,
    carry: Carry,
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
            probe: None,
            carry: Carry::default(),
        }
    }

    /// Holds the pump at the named points of `probe`. A test seam: production never sets one
    /// (XR-021 CONTRACTS.md R6-03 rule 7).
    #[must_use]
    pub fn with_publish_probe(mut self, probe: std::sync::Arc<PublishProbe>) -> Self {
        self.probe = Some(probe);
        self
    }

    async fn pause_at(&self, point: PublishPause) {
        if let Some(probe) = &self.probe {
            probe.pause_at(point).await;
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
        self.reconcile_carried().await?;
        let candidates = self.pending_rows().await?;
        if candidates.is_empty() {
            return Ok(0);
        }
        self.pause_at(PublishPause::AfterCandidates).await;
        let connection = self.connection(endpoint, nkey_seed_path).await?;
        self.publish_rows(&connection, candidates).await
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

    async fn pending_rows(&self) -> Result<Vec<Candidate>, PublishError> {
        // First publications, and the published tombstones whose acknowledgement is overdue
        // (XR-021 CONTRACTS.md R5-11): the second kind is published again, unchanged. The batch is
        // chosen without a lock; a first publication takes its row lock later, one row at a time.
        let rows: Vec<sqlx::postgres::PgRow> = sqlx::query(
            "SELECT id, event_type, payload, tenant_id, published_at IS NOT NULL AS resend
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
        .map_err(PublishError::Database)?;
        rows.iter().map(Candidate::read).collect()
    }
}

/// A row chosen for this pass, before any lock is taken on it.
#[derive(Debug)]
struct Candidate {
    id: i64,
    event_type: String,
    payload: serde_json::Value,
    /// The archive account the row is bound to.
    account_id: Option<uuid::Uuid>,
    /// Whether the row is an already published tombstone that is published again.
    resend: bool,
}

impl Candidate {
    fn read(row: &sqlx::postgres::PgRow) -> Result<Self, PublishError> {
        use sqlx::Row as _;
        Ok(Self {
            id: row.try_get("id").map_err(PublishError::Database)?,
            event_type: row.try_get("event_type").map_err(PublishError::Database)?,
            payload: row.try_get("payload").map_err(PublishError::Database)?,
            account_id: row.try_get("tenant_id").map_err(PublishError::Database)?,
            resend: row.try_get("resend").map_err(PublishError::Database)?,
        })
    }
}
