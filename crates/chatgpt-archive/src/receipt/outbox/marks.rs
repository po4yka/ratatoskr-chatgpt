//! The short statements that move an outbox row between its delivery states.

use std::time::Duration;

use sqlx::{Postgres, Transaction};

use crate::deletion_ack::TOMBSTONE_EVENT_TYPE;

use super::{ArchiveEventOutbox, CLASS_PAYLOAD_TOO_LARGE, MAX_BACKOFF_SECONDS, PublishError};

/// Marks a first publication inside the transaction that holds the row, and records the stream
/// sequences of the broker acknowledgements (XR-021 CONTRACTS.md R6-03 rules 1 and 2).
///
/// `sequences` are the acknowledged sequence and any carried from earlier acknowledgements whose
/// commit failed; they are merged with the array already on the row, ascending and without
/// duplicates. A tombstone also schedules its first resend, which is cancelled by Knowledge's
/// acknowledgement.
pub(super) async fn mark_published(
    transaction: &mut Transaction<'_, Postgres>,
    id: i64,
    sequences: &[i64],
    resend_after: Duration,
) -> Result<(), PublishError> {
    let marked = sqlx::query(
        "UPDATE chatgpt_archive.outbox_events
         SET published_at = now(),
             published_stream_seqs = ARRAY(
                 SELECT DISTINCT sequence
                 FROM unnest(published_stream_seqs || $4::bigint[]) AS sequence
                 ORDER BY sequence),
             next_resend_at = CASE WHEN event_type = $2
                                   THEN now() + make_interval(secs => $3) END
         WHERE id = $1 AND published_at IS NULL",
    )
    .bind(id)
    .bind(TOMBSTONE_EVENT_TYPE)
    .bind(resend_after.as_secs_f64())
    .bind(sequences)
    .execute(&mut **transaction)
    .await
    .map_err(PublishError::Database)?;
    if marked.rows_affected() != 1 {
        // The broker holds a copy no row records. The caller carries the sequence, so the copy is
        // erased if the row is gone, and the operator sees the counter move.
        metrics::counter!("chatgpt_archive_outbox_mark_lost_total").increment(1);
        tracing::error!(
            rows = marked.rows_affected(),
            "the mark of an acknowledged archive event did not land on exactly one row"
        );
        return Err(PublishError::MarkLost);
    }
    Ok(())
}

impl ArchiveEventOutbox {
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use sqlx::PgPool;

    use super::{PublishError, mark_published};

    const SCHEMA_SQL: &str = include_str!("../../../../../schema.sql");

    /// A database that exists for this test only, created from the current `schema.sql`.
    async fn disposable() -> (PgPool, PgPool, String) {
        #[allow(
            clippy::disallowed_methods,
            reason = "the test harness reads the PostgreSQL URL the runner exports"
        )]
        let admin_url = std::env::var("CHATGPT_TEST_DATABASE_URL")
            .expect("CHATGPT_TEST_DATABASE_URL must point at a PostgreSQL 17 server");
        let admin = PgPool::connect(&admin_url)
            .await
            .expect("the administrative database accepts connections");
        let name = format!("xr021_marks_{}", uuid::Uuid::now_v7().simple());
        sqlx::query(&format!("CREATE DATABASE {name}"))
            .execute(&admin)
            .await
            .expect("a disposable database can be created");
        let (server, _) = admin_url
            .rsplit_once('/')
            .expect("the database URL names a database");
        let pool = PgPool::connect(&format!("{server}/{name}"))
            .await
            .expect("the disposable database accepts connections");
        sqlx::raw_sql(SCHEMA_SQL)
            .execute(&pool)
            .await
            .expect("the current schema applies");
        (admin, pool, name)
    }

    async fn insert_row(pool: &PgPool, published: bool) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO chatgpt_archive.outbox_events
             (event_type, aggregate_id, payload, published_at)
             VALUES ('ai_archive.conversation.added.v1', gen_random_uuid(), '{}',
                     CASE WHEN $1 THEN now() END)
             RETURNING id",
        )
        .bind(published)
        .fetch_one(pool)
        .await
        .expect("the row is inserted")
    }

    fn counter(handle: &metrics_exporter_prometheus::PrometheusHandle) -> f64 {
        handle
            .render()
            .lines()
            .find_map(|line| {
                line.strip_prefix("chatgpt_archive_outbox_mark_lost_total")?
                    .trim()
                    .parse::<f64>()
                    .ok()
            })
            .unwrap_or(0.0)
    }

    #[tokio::test]
    async fn a_mark_that_lands_nowhere_is_a_counted_error() {
        let handle = metrics_exporter_prometheus::PrometheusBuilder::new()
            .install_recorder()
            .expect("no other recorder is installed in this test binary");
        let (admin, pool, name) = disposable().await;
        let published = insert_row(&pool, true).await;
        let after = Duration::from_mins(1);

        let mut transaction = pool.begin().await.expect("a transaction");
        let missing = mark_published(&mut transaction, published + 1000, &[7], after).await;
        assert!(
            matches!(missing, Err(PublishError::MarkLost)),
            "a row that no longer exists is a lost mark, got {missing:?}"
        );
        assert!(
            (counter(&handle) - 1.0).abs() < f64::EPSILON,
            "counted once"
        );

        let again = mark_published(&mut transaction, published, &[7], after).await;
        assert!(
            matches!(again, Err(PublishError::MarkLost)),
            "an already published row is a lost mark, got {again:?}"
        );
        assert!(
            (counter(&handle) - 2.0).abs() < f64::EPSILON,
            "counted twice"
        );
        transaction.rollback().await.expect("the rollback succeeds");

        pool.close().await;
        sqlx::query(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
            .execute(&admin)
            .await
            .expect("the disposable database can be dropped");
    }
}
