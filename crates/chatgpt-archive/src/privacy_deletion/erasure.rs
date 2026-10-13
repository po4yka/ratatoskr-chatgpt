//! Asking Edge to erase the stream copies of the facts a deletion removed from the outbox
//! (XR-021 CONTRACTS.md R5-12 rule 3 and R6-03 rule 3).
//!
//! Conversation, project and archive facts carry message text on the wire and stay in
//! `ratatoskr_events` after this service deletes its own rows. No producer may manage the stream,
//! so the deletion reads the stream sequences of every published fact in the statement that
//! removes it and, in the same transaction, enqueues `platform.stream_erasure.requested.v1` for
//! Edge, which erases only messages this service published for the owner the command names.

use std::collections::{BTreeMap, BTreeSet};

use ratatoskr_identifiers::WireTimestamp;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::execution::DurableRequest;
use super::service::PrivacyDeletionError;
use crate::stream_erasure::{
    CONTENT_BEARING_TYPES, EnqueueError, OwnerErasure, enqueue_owner_erasure,
};

type Tx<'a> = Transaction<'a, Postgres>;

/// The stream sequences of the published facts a deletion removed, by owner (the envelope tenant).
/// A set is ascending by construction.
#[derive(Debug, Default)]
pub(super) struct PublishedFacts {
    by_owner: BTreeMap<String, BTreeSet<i64>>,
}

/// Deletes the outbox rows named by `outbox_ids` and every row bound to one of `exports`, and
/// returns the stream sequences of the published, content-bearing facts among them.
///
/// This is the one place a published fact row leaves the outbox during a deletion: the inventory
/// removal, the export cascade and the tenant-scope removal all go through it, and the sequences
/// come from the same statement that deletes the rows, so none can be missed between collecting
/// and deleting. A row that was never published has no sequence and needs nothing.
pub(super) async fn drop_published_fact_rows(
    transaction: &mut Tx<'_>,
    outbox_ids: &[i64],
    exports: &[Uuid],
) -> Result<PublishedFacts, sqlx::Error> {
    let types: Vec<String> = CONTENT_BEARING_TYPES
        .iter()
        .map(|value| (*value).to_owned())
        .collect();
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "WITH dropped AS (
           DELETE FROM chatgpt_archive.outbox_events
           WHERE id = ANY($1) OR export_id = ANY($2)
           RETURNING event_type, payload ->> 'tenant_id' AS owner, published_stream_seqs
         )
         SELECT owner, unnest(published_stream_seqs) FROM dropped
         WHERE event_type = ANY($3) AND owner IS NOT NULL",
    )
    .bind(outbox_ids)
    .bind(exports)
    .bind(&types)
    .fetch_all(&mut **transaction)
    .await?;
    let mut facts = PublishedFacts::default();
    for (owner, sequence) in rows {
        facts.by_owner.entry(owner).or_default().insert(sequence);
    }
    Ok(facts)
}

/// Enqueues the erasure commands of a deletion, in the transaction that removed the rows: one
/// command per distinct owner, at most 500 ascending sequences each. Nothing is enqueued when no
/// published fact was removed.
pub(super) async fn enqueue_erasure_commands(
    transaction: &mut Tx<'_>,
    durable: &DurableRequest,
    request_id: Uuid,
    issued_at: WireTimestamp,
    facts: &PublishedFacts,
) -> Result<(), PrivacyDeletionError> {
    for (owner, sequences) in &facts.by_owner {
        let key_stem = format!("stream-erasure:{request_id}:{owner}");
        enqueue_owner_erasure(
            transaction,
            &OwnerErasure {
                owner,
                sequences,
                request_id,
                account_id: Some(durable.tenant_id),
                correlation_id: durable.correlation_id,
                issued_at,
                key_stem: &key_stem,
            },
        )
        .await
        .map_err(|error| match error {
            EnqueueError::Invalid => PrivacyDeletionError::StreamErasure,
            EnqueueError::Database(source) => PrivacyDeletionError::Store(source),
            EnqueueError::Encode(source) => PrivacyDeletionError::Encode(source),
        })?;
    }
    Ok(())
}
