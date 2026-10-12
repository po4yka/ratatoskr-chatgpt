//! Knowledge's acknowledgement of deletion tombstones (XR-021 CONTRACTS.md R5-11 rule 4).
//!
//! The events stream keeps dropping its oldest message at 7 days or 1 GiB, and a producer marks a
//! tombstone published at the broker acknowledgement. So a tombstone Knowledge did not consume in
//! time would be lost silently. The pump therefore re-sends a published tombstone until Knowledge
//! publishes `knowledge.deletion.acknowledged.v1` for it, and the consumer in [`consumer`] records
//! that acknowledgement on the outbox row.

use std::time::Duration;

use ratatoskr_operation_contracts::{DeletionAckOutcome, KnowledgeDeletionAcknowledged};
use serde::Serialize;
use sqlx::PgPool;

pub mod consumer;

/// The event type of the tombstone rows that use the acknowledgement columns.
pub(crate) const TOMBSTONE_EVENT_TYPE: &str = "ai_archive.subject.tombstoned.v1";

/// The default resend interval in seconds.
pub const DEFAULT_RESEND_AFTER_SECONDS: u64 = 600;

/// The default resend cap in seconds.
pub const DEFAULT_RESEND_CAP_SECONDS: u64 = 21_600;

/// How long a published tombstone waits for its acknowledgement before it is published again,
/// and how far the wait backs off.
///
/// The events stream collapses a repeated `Nats-Msg-Id` inside its 120 second duplicate window,
/// so a resend sooner than that is a no-op at the broker; the library accepts any value so a test
/// can run against a stream with a shorter window, and the configuration loader enforces the
/// operator range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TombstoneResend {
    /// The wait before the first resend; it doubles with every resend.
    pub after: Duration,
    /// The longest wait between two resends.
    pub cap: Duration,
}

impl Default for TombstoneResend {
    fn default() -> Self {
        Self {
            after: Duration::from_secs(DEFAULT_RESEND_AFTER_SECONDS),
            cap: Duration::from_secs(DEFAULT_RESEND_CAP_SECONDS),
        }
    }
}

/// What an acknowledgement did to the outbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckApplication {
    /// The tombstone was acknowledged as applied and will not be re-sent.
    Applied,
    /// Knowledge refused the tombstone; the resend stopped and an operator must look.
    Refused,
    /// No unacknowledged tombstone of this owner has that event id: an unknown id, another
    /// owner's row, a row of another service, or a duplicate. Nothing changed.
    Unmatched,
}

/// Records Knowledge's acknowledgement on the tombstone it names.
///
/// The row is matched by the event id of its stored envelope and by the owner, which is the
/// envelope tenant, so an acknowledgement for another owner's row or for no row changes nothing,
/// and a duplicate changes nothing either. A `refused` acknowledgement also ends the resend, and
/// raises an ERROR line and `chatgpt_archive_tombstone_refused_total`: the data stays in
/// Knowledge and an operator must look. The acknowledgement sets `published_at` when the row has
/// none, so an acknowledged row is never published a second time.
///
/// # Errors
///
/// Returns the database error when the outbox cannot be updated.
pub async fn acknowledge(
    pool: &PgPool,
    acknowledgement: &KnowledgeDeletionAcknowledged,
) -> Result<AckApplication, sqlx::Error> {
    let refused = matches!(acknowledgement.outcome, DeletionAckOutcome::Refused);
    let updated = sqlx::query(
        "UPDATE chatgpt_archive.outbox_events
         SET acknowledged_at = now(), ack_outcome = $3,
             published_at = COALESCE(published_at, now())
         WHERE event_type = $4 AND acknowledged_at IS NULL
           AND payload ->> 'event_id' = $1 AND payload ->> 'tenant_id' = $2",
    )
    .bind(acknowledgement.acknowledged_event_id.to_string())
    .bind(acknowledgement.owner.to_string())
    .bind(if refused { "refused" } else { "applied" })
    .bind(TOMBSTONE_EVENT_TYPE)
    .execute(pool)
    .await?;
    if updated.rows_affected() == 0 {
        return Ok(AckApplication::Unmatched);
    }
    if refused {
        metrics::counter!("chatgpt_archive_tombstone_refused_total").increment(1);
        tracing::error!(
            event_id = %acknowledgement.acknowledged_event_id,
            "Knowledge refused a deletion tombstone; the data stays in Knowledge and an operator \
             must look"
        );
        return Ok(AckApplication::Refused);
    }
    Ok(AckApplication::Applied)
}

/// Refreshes the gauges of the tombstones Knowledge has not acknowledged: how many there are and
/// how old the oldest is. Readiness is deliberately not tied to them: a Knowledge outage must not
/// make this producer unready.
///
/// # Errors
///
/// Returns the database error when the outbox cannot be read.
pub async fn refresh_gauges(pool: &PgPool) -> Result<(), sqlx::Error> {
    let (count, oldest_seconds): (f64, f64) = sqlx::query_as(
        "SELECT count(*)::float8,
                COALESCE(EXTRACT(EPOCH FROM (now() - min(created_at))), 0)::float8
         FROM chatgpt_archive.outbox_events
         WHERE event_type = $1 AND acknowledged_at IS NULL",
    )
    .bind(TOMBSTONE_EVENT_TYPE)
    .fetch_one(pool)
    .await?;
    metrics::gauge!("chatgpt_archive_unacknowledged_tombstones").set(count);
    metrics::gauge!("chatgpt_archive_oldest_unacknowledged_tombstone_age_seconds")
        .set(oldest_seconds);
    Ok(())
}
