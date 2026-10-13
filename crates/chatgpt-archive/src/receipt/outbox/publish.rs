//! The publication of one batch: a first publication under its row lock, and the resend of an
//! unacknowledged tombstone.

use sqlx::{Postgres, Transaction};

use super::subject::{publish_row, stored_envelope, subject_for_event_type};
use super::{
    ArchiveEventOutbox, Candidate, Connection, PAYLOAD_HEADROOM_BYTES, PUBLISH_TIMEOUT,
    PublishError, marks,
};
use crate::receipt::probe::PublishPause;

type Tx<'a> = Transaction<'a, Postgres>;

/// What happened to one candidate row.
enum Outcome {
    /// The broker acknowledged the message and its sequences are recorded.
    Published,
    /// The row was gone or held by a deletion; nothing was published.
    Skipped,
}

impl ArchiveEventOutbox {
    /// Publishes `candidates` in id order and marks only broker-acknowledged rows.
    ///
    /// Each message is named by its envelope `event_id`, so a crash after
    /// acknowledgement but before the SQL commit redelivers a message the stream
    /// collapses, and the key space cannot collide with another producer's. A row the
    /// broker did not acknowledge stays unpublished, records a safe failure class and
    /// waits out a bounded backoff, so one refused row never starves the rows behind
    /// it. A transient failure never gives a row up. A row larger than the connected
    /// server accepts can never be delivered: it is marked undeliverable at once,
    /// skipped from then on, counted, and does not fail the pass, so it neither wedges
    /// the pump nor flips readiness.
    ///
    /// A first publication is one short transaction per row that locks the row, publishes,
    /// marks and commits (XR-021 CONTRACTS.md R6-03 rule 2): a row a deletion removed or holds is
    /// skipped and nothing is published for it, and a deletion that meets the lock waits for the
    /// pump.
    ///
    /// # Errors
    ///
    /// Returns [`PublishError::UnknownEventType`] or [`PublishError::InvalidRow`]
    /// for a row this service cannot have produced; both stop the pass without
    /// publishing that row. Returns [`PublishError::Broker`] when any row was not
    /// acknowledged, and [`PublishError::MarkLost`] when an acknowledged row could not
    /// be marked. A `PubAck` timeout is indistinguishable from a permission
    /// denial, so check the NATS server log for a Publish Violation.
    pub(super) async fn publish_rows(
        &self,
        connection: &Connection,
        candidates: Vec<Candidate>,
    ) -> Result<usize, PublishError> {
        let jetstream = async_nats::jetstream::ContextBuilder::new()
            .timeout(PUBLISH_TIMEOUT)
            .build(connection.client.clone());
        let deliverable_bytes = connection
            .client
            .server_info()
            .max_payload
            .saturating_sub(PAYLOAD_HEADROOM_BYTES);
        let mut published = 0;
        let mut first_failure = None;
        for candidate in candidates {
            let subject = subject_for_event_type(&candidate.event_type)
                .ok_or(PublishError::UnknownEventType)?;
            let (body, event_id) = stored_envelope(&candidate.event_type, &candidate.payload)?;
            if body.len() > deliverable_bytes {
                self.mark_undeliverable(candidate.id).await?;
                continue;
            }
            let outcome = if candidate.resend {
                self.publish_again(&jetstream, &candidate, subject, &event_id, body)
                    .await
            } else {
                self.publish_first(&jetstream, &candidate, subject, &event_id, body)
                    .await
            };
            match outcome {
                Ok(Outcome::Published) => published += 1,
                Ok(Outcome::Skipped) => {
                    metrics::counter!("chatgpt_archive_outbox_skipped_total").increment(1);
                }
                Err(error @ PublishError::Broker(_)) => {
                    if error.is_closed_connection() {
                        connection.mark_closed();
                    }
                    if candidate.resend {
                        self.record_resend_failure(candidate.id).await?;
                    } else {
                        self.record_failure(candidate.id).await?;
                    }
                    first_failure.get_or_insert(error);
                }
                // The copy is carried; the pass fails and the next one reconciles it.
                Err(error @ PublishError::MarkLost) => {
                    first_failure.get_or_insert(error);
                }
                Err(error) => return Err(error),
            }
        }
        first_failure.map_or(Ok(published), Err)
    }

    /// Publishes a tombstone whose acknowledgement is overdue, unchanged, and books the next
    /// resend. The row records no sequence and takes no lock.
    async fn publish_again(
        &self,
        jetstream: &async_nats::jetstream::Context,
        candidate: &Candidate,
        subject: &'static str,
        event_id: &str,
        body: Vec<u8>,
    ) -> Result<Outcome, PublishError> {
        publish_row(jetstream, subject, event_id, body).await?;
        self.mark_resent(candidate.id).await?;
        Ok(Outcome::Published)
    }

    /// The first publication of a row, in one transaction that holds the row lock from before the
    /// publish until after the commit.
    ///
    /// The transaction is always ended explicitly before this returns: a dropped transaction would
    /// leave the lock until its connection is reused, and the failure bookkeeping that follows
    /// updates the same row from another connection.
    async fn publish_first(
        &self,
        jetstream: &async_nats::jetstream::Context,
        candidate: &Candidate,
        subject: &'static str,
        event_id: &str,
        body: Vec<u8>,
    ) -> Result<Outcome, PublishError> {
        let mut transaction = self.pool.begin().await.map_err(PublishError::Database)?;
        let result = self
            .publish_locked(
                &mut transaction,
                jetstream,
                candidate,
                subject,
                event_id,
                body,
            )
            .await;
        match result {
            Ok(Outcome::Published) => {
                self.commit(transaction).await?;
                self.carry.forget(candidate.id);
                Ok(Outcome::Published)
            }
            other => {
                // A failed rollback means the session is gone, which releases the lock anyway.
                let _ = transaction.rollback().await;
                other
            }
        }
    }

    async fn publish_locked(
        &self,
        transaction: &mut Tx<'_>,
        jetstream: &async_nats::jetstream::Context,
        candidate: &Candidate,
        subject: &'static str,
        event_id: &str,
        body: Vec<u8>,
    ) -> Result<Outcome, PublishError> {
        // `skip locked`: the pump never waits on a lock, so it cannot take part in a deadlock. A
        // row that is gone, already published or held by a deletion is simply not published.
        let locked: Option<i32> = sqlx::query_scalar(
            "SELECT 1 FROM chatgpt_archive.outbox_events
             WHERE id = $1 AND published_at IS NULL AND undeliverable_at IS NULL
             FOR UPDATE SKIP LOCKED",
        )
        .bind(candidate.id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(PublishError::Database)?;
        if locked.is_none() {
            tracing::debug!("an archive event was deleted or is held; it is not published");
            return Ok(Outcome::Skipped);
        }
        let sequence = publish_row(jetstream, subject, event_id, body).await?;
        self.pause_at(PublishPause::AfterPubAck).await;
        // From here until the commit lands, the copy exists on the stream and no row records it.
        let sequences = self.carry.remember(candidate, event_id, sequence)?;
        marks::mark_published(transaction, candidate.id, &sequences, self.resend.after).await?;
        Ok(Outcome::Published)
    }

    /// Commits an acknowledged publication, unless a test probe fails this commit.
    async fn commit(&self, transaction: Transaction<'_, Postgres>) -> Result<(), PublishError> {
        if self
            .probe
            .as_ref()
            .is_some_and(|probe| probe.take_commit_failure())
        {
            transaction
                .rollback()
                .await
                .map_err(PublishError::Database)?;
            return Err(PublishError::Database(sqlx::Error::Protocol(
                "the commit was failed by a test probe".into(),
            )));
        }
        transaction.commit().await.map_err(PublishError::Database)
    }
}
