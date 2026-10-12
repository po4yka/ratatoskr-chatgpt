//! Asking Edge to erase the stream copies of the facts a deletion removed from the outbox
//! (XR-021 CONTRACTS.md R5-12 rules 2 and 3).
//!
//! Conversation, project and archive facts carry message text on the wire and stay in
//! `ratatoskr_events` after this service deletes its own rows. No producer may manage the stream,
//! so the deletion collects the stream sequence of every published fact it removes and, in the same
//! transaction, enqueues `platform.stream_erasure.requested.v1` for Edge, which erases only
//! messages this service published for the owner the command names.

use std::collections::{BTreeMap, BTreeSet};

use ratatoskr_event_envelope::{
    CommandEnvelope, CommandPayload as _, EnvelopeSchemaVersion, ProducerName,
};
use ratatoskr_identifiers::{
    CommandId, CorrelationId, EntityKind, EntityLocalId, EntityRef, Extensions, TenantRef,
    WireTimestamp,
};
use ratatoskr_operation_contracts::PlatformStreamErasureRequested;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::execution::DurableRequest;
use super::service::PrivacyDeletionError;

type Tx<'a> = Transaction<'a, Postgres>;

/// The event types of the facts that carry content on the wire. Never the tombstone, an
/// operation report or an erasure command.
const CONTENT_BEARING_TYPES: [&str; 7] = [
    "ai_archive.archive.imported.v1",
    "ai_archive.conversation.added.v1",
    "ai_archive.conversation.updated.v1",
    "ai_archive.project.added.v1",
    "ai_archive.project.updated.v1",
    "ai_archive.artifact.added.v1",
    "ai_archive.artifact.updated.v1",
];

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
           RETURNING event_type, payload ->> 'tenant_id' AS owner, published_stream_seq
         )
         SELECT owner, published_stream_seq FROM dropped
         WHERE event_type = ANY($3) AND owner IS NOT NULL AND published_stream_seq IS NOT NULL",
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

/// The most sequences one erasure command may name.
const MAX_SEQUENCES_PER_COMMAND: usize = 500;

/// The deployable that asks.
const PRODUCER: &str = "ratatoskr-chatgpt";

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
    let request_ref = EntityRef::new(
        EntityKind::parse("privacy_request").map_err(|_| PrivacyDeletionError::StreamErasure)?,
        EntityLocalId::parse(&request_id.to_string())
            .map_err(|_| PrivacyDeletionError::StreamErasure)?,
    );
    let producer =
        ProducerName::parse(PRODUCER).map_err(|_| PrivacyDeletionError::StreamErasure)?;
    for (owner, sequences) in &facts.by_owner {
        let owner_ref = TenantRef::parse(owner).map_err(|_| PrivacyDeletionError::StreamErasure)?;
        let sequences: Vec<u64> = sequences
            .iter()
            .map(|sequence| u64::try_from(*sequence))
            .collect::<Result<_, _>>()
            .map_err(|_| PrivacyDeletionError::StreamErasure)?;
        for (index, chunk) in sequences.chunks(MAX_SEQUENCES_PER_COMMAND).enumerate() {
            let command_id = CommandId::new_v7();
            let mut envelope = CommandEnvelope {
                command_id,
                command_type: PlatformStreamErasureRequested::command_type(),
                issued_at,
                producer: producer.clone(),
                aggregate_id: request_ref.clone(),
                correlation_id: durable.correlation_id.map_or_else(
                    || command_id.as_entity_ref(),
                    |id| CorrelationId(id).as_entity_ref(),
                ),
                causation_id: None,
                tenant_id: Some(owner_ref),
                schema_version: EnvelopeSchemaVersion::CURRENT,
                payload: serde_json::Map::new(),
                extensions: Extensions::new(),
            };
            envelope
                .set_payload(&PlatformStreamErasureRequested {
                    request_ref: request_ref.clone(),
                    owner: owner_ref,
                    stream_sequences: chunk.to_vec(),
                    extensions: Extensions::new(),
                })
                .map_err(|_| PrivacyDeletionError::StreamErasure)?;
            sqlx::query(
                "INSERT INTO chatgpt_archive.outbox_events
                 (event_type, aggregate_id, tenant_id, export_id, payload, correlation_id,
                  deduplication_key)
                 VALUES ($1, $2, $3, NULL, $4, $5, $6)
                 ON CONFLICT (deduplication_key) WHERE deduplication_key IS NOT NULL DO NOTHING",
            )
            .bind(PlatformStreamErasureRequested::COMMAND_TYPE)
            .bind(request_id)
            .bind(durable.tenant_id)
            .bind(serde_json::to_value(&envelope)?)
            .bind(envelope.correlation_id.as_uuid())
            .bind(format!("stream-erasure:{request_id}:{owner}:{index}"))
            .execute(&mut **transaction)
            .await?;
        }
    }
    Ok(())
}
