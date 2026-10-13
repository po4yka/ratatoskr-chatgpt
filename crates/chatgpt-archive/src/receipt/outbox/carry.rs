//! The copies the broker acknowledged and the database never recorded (XR-021 CONTRACTS.md
//! R6-03 rule 4).
//!
//! A first publication that fails between the broker's `PubAck` and the commit of its sequence
//! leaves a stream message that no row names. The pump keeps that sequence in memory: the next
//! successful mark of the row merges it into `published_stream_seqs`, and when the row is gone
//! before that, no deletion knew about the copy, so the pump asks Edge to erase it itself.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use uuid::Uuid;

use super::{ArchiveEventOutbox, Candidate, PublishError};
use crate::pg_instant::database_now;
use crate::stream_erasure::{
    CONTENT_BEARING_TYPES, EnqueueError, OwnerErasure, enqueue_owner_erasure,
};

/// The acknowledged sequences of one row whose commit has not landed.
#[derive(Debug, Clone)]
struct Carried {
    owner: String,
    event_id: Uuid,
    account_id: Option<Uuid>,
    sequences: BTreeSet<i64>,
}

/// Row id to carried copies. A std mutex that is never held across an await.
#[derive(Debug, Clone, Default)]
pub(super) struct Carry {
    entries: Arc<Mutex<HashMap<i64, Carried>>>,
}

impl Carry {
    fn lock(&self) -> MutexGuard<'_, HashMap<i64, Carried>> {
        // A panic while the map was held cannot leave it half-updated: every update is one call.
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Notes the acknowledgement `sequence` of `candidate` and returns every sequence the row must
    /// record when its mark lands: this one and the ones carried from earlier acknowledgements.
    ///
    /// Only the content-bearing facts are carried. An operation report, a tombstone and an erasure
    /// command are never erased from the stream, so a sequence of theirs would only make Edge
    /// count a refusal.
    pub(super) fn remember(
        &self,
        candidate: &Candidate,
        event_id: &str,
        sequence: u64,
    ) -> Result<Vec<i64>, PublishError> {
        let sequence = i64::try_from(sequence).map_err(|_| PublishError::InvalidRow)?;
        let owner = candidate
            .payload
            .get("tenant_id")
            .and_then(serde_json::Value::as_str);
        let Some(owner) =
            owner.filter(|_| CONTENT_BEARING_TYPES.contains(&candidate.event_type.as_str()))
        else {
            return Ok(vec![sequence]);
        };
        let event_id = Uuid::parse_str(event_id).map_err(|_| PublishError::InvalidRow)?;
        let mut entries = self.lock();
        let carried = entries.entry(candidate.id).or_insert_with(|| Carried {
            owner: owner.to_owned(),
            event_id,
            account_id: candidate.account_id,
            sequences: BTreeSet::new(),
        });
        carried.sequences.insert(sequence);
        Ok(carried.sequences.iter().copied().collect())
    }

    /// Drops the entry of a row whose sequences are recorded, or whose command is queued.
    pub(super) fn forget(&self, id: i64) {
        self.lock().remove(&id);
    }

    fn snapshot(&self) -> Vec<(i64, Carried)> {
        self.lock()
            .iter()
            .map(|(id, carried)| (*id, carried.clone()))
            .collect()
    }
}

impl ArchiveEventOutbox {
    /// Turns every carried copy whose row is gone into an erasure command in the owner's name.
    ///
    /// The row was deleted before its retry, so the deletion found no sequence to name. The
    /// command's `request_ref` is `privacy_request:<event id of the orphaned row>`, which names no
    /// real deletion request; the deduplication key is `stream-erasure:reconcile:<event id>:<n>`,
    /// so a later pass enqueues nothing more.
    pub(super) async fn reconcile_carried(&self) -> Result<(), PublishError> {
        let carried = self.carry.snapshot();
        if carried.is_empty() {
            return Ok(());
        }
        let ids: Vec<i64> = carried.iter().map(|(id, _)| *id).collect();
        let present: Vec<i64> =
            sqlx::query_scalar("SELECT id FROM chatgpt_archive.outbox_events WHERE id = ANY($1)")
                .bind(&ids)
                .fetch_all(&self.pool)
                .await
                .map_err(PublishError::Database)?;
        for (id, entry) in carried.into_iter().filter(|(id, _)| !present.contains(id)) {
            let mut transaction = self.pool.begin().await.map_err(PublishError::Database)?;
            let issued_at = database_now(&mut transaction)
                .await
                .map_err(PublishError::Database)?;
            let key_stem = format!("stream-erasure:reconcile:{}", entry.event_id);
            enqueue_owner_erasure(
                &mut transaction,
                &OwnerErasure {
                    owner: &entry.owner,
                    sequences: &entry.sequences,
                    request_id: entry.event_id,
                    account_id: entry.account_id,
                    correlation_id: None,
                    issued_at,
                    key_stem: &key_stem,
                },
            )
            .await
            .map_err(|error| match error {
                EnqueueError::Invalid => PublishError::InvalidRow,
                EnqueueError::Database(source) => PublishError::Database(source),
                EnqueueError::Encode(source) => PublishError::Encode(source),
            })?;
            transaction.commit().await.map_err(PublishError::Database)?;
            self.carry.forget(id);
            tracing::warn!(
                copies = entry.sequences.len(),
                "an archive event was deleted before its acknowledged copy was recorded; \
                 the pump asked Edge to erase the copy"
            );
        }
        Ok(())
    }
}
