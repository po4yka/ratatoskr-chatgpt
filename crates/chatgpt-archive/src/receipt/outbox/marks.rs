//! The short statements that move an outbox row between its delivery states.

use crate::deletion_ack::TOMBSTONE_EVENT_TYPE;

use super::{ArchiveEventOutbox, CLASS_PAYLOAD_TOO_LARGE, MAX_BACKOFF_SECONDS, PublishError};

impl ArchiveEventOutbox {
    /// Marks a first publication and records the stream sequence of the broker acknowledgement
    /// (XR-021 CONTRACTS.md R5-12). A tombstone also schedules its first resend, which is
    /// cancelled by Knowledge's acknowledgement.
    pub(super) async fn mark_published(&self, id: i64, sequence: u64) -> Result<(), PublishError> {
        sqlx::query(
            "UPDATE chatgpt_archive.outbox_events
             SET published_at = now(),
                 published_stream_seqs = ARRAY(
                     SELECT DISTINCT sequence
                     FROM unnest(published_stream_seqs || $4::bigint) AS sequence
                     ORDER BY sequence),
                 next_resend_at = CASE WHEN event_type = $2
                                       THEN now() + make_interval(secs => $3) END
             WHERE id = $1 AND published_at IS NULL",
        )
        .bind(id)
        .bind(TOMBSTONE_EVENT_TYPE)
        .bind(self.resend.after.as_secs_f64())
        .bind(i64::try_from(sequence).unwrap_or(i64::MAX))
        .execute(&self.pool)
        .await
        .map_err(PublishError::Database)?;
        Ok(())
    }

    /// Books a resend: after the n-th resend the next one waits `after * 2^n`, never longer than the
    /// cap, so the wait doubles every time.
    pub(super) async fn mark_resent(&self, id: i64) -> Result<(), PublishError> {
        sqlx::query(
            "UPDATE chatgpt_archive.outbox_events
             SET next_resend_at = now() + make_interval(
                     secs => least($2::float8 * power(2, least(resend_count + 1, 30)), $3::float8)),
                 resend_count = resend_count + 1
             WHERE id = $1 AND acknowledged_at IS NULL",
        )
        .bind(id)
        .bind(self.resend.after.as_secs_f64())
        .bind(self.resend.cap.as_secs_f64())
        .execute(&self.pool)
        .await
        .map_err(PublishError::Database)?;
        Ok(())
    }

    /// A republish the broker did not acknowledge: the row stays as it is and the existing
    /// failure backoff delays the next attempt. It is not a resend.
    pub(super) async fn record_resend_failure(&self, id: i64) -> Result<(), PublishError> {
        sqlx::query(
            "UPDATE chatgpt_archive.outbox_events
             SET attempt_count = attempt_count + 1, last_error = 'not_acknowledged',
                 next_resend_at = now() + make_interval(
                     secs => least(power(2, least(attempt_count, 8))::int, $2))
             WHERE id = $1 AND published_at IS NOT NULL AND acknowledged_at IS NULL",
        )
        .bind(id)
        .bind(MAX_BACKOFF_SECONDS)
        .execute(&self.pool)
        .await
        .map_err(PublishError::Database)?;
        Ok(())
    }

    /// A permanent failure: the row is kept as evidence and never retried. The log and the
    /// counter carry the class only, never the event type's content.
    pub(super) async fn mark_undeliverable(&self, id: i64) -> Result<(), PublishError> {
        let marked = sqlx::query(
            "UPDATE chatgpt_archive.outbox_events
             SET undeliverable_at = now(), last_error = $2
             WHERE id = $1 AND published_at IS NULL AND undeliverable_at IS NULL",
        )
        .bind(id)
        .bind(CLASS_PAYLOAD_TOO_LARGE)
        .execute(&self.pool)
        .await
        .map_err(PublishError::Database)?;
        if marked.rows_affected() > 0 {
            metrics::counter!(
                "chatgpt_archive_outbox_undeliverable_total",
                "class" => CLASS_PAYLOAD_TOO_LARGE
            )
            .increment(1);
            tracing::warn!(
                class = CLASS_PAYLOAD_TOO_LARGE,
                "an archive event is larger than the broker accepts and will not be retried"
            );
        }
        Ok(())
    }

    pub(super) async fn record_failure(&self, id: i64) -> Result<(), PublishError> {
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
