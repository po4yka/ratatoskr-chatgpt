//! The request that asks Edge to erase stream messages, built in one place for its two issuers: a
//! deletion that removed published fact rows, and the pump that published a copy no deletion
//! knew about (XR-021 CONTRACTS.md R5-12 rule 3 and R6-03 rules 3 and 4).
//!
//! Conversation, project and archive facts carry message text on the wire and stay in
//! `ratatoskr_events` after this service deletes its own rows. No producer may manage the stream,
//! so a producer enqueues `platform.stream_erasure.requested.v1` for Edge, which erases only
//! messages this service published for the owner the command names.

use std::collections::BTreeSet;

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

/// The event types of the facts that carry content on the wire. Never the tombstone, an
/// operation report or an erasure command.
pub(crate) const CONTENT_BEARING_TYPES: [&str; 7] = [
    "ai_archive.archive.imported.v1",
    "ai_archive.conversation.added.v1",
    "ai_archive.conversation.updated.v1",
    "ai_archive.project.added.v1",
    "ai_archive.project.updated.v1",
    "ai_archive.artifact.added.v1",
    "ai_archive.artifact.updated.v1",
];

/// The most sequences one erasure command may name.
const MAX_SEQUENCES_PER_COMMAND: usize = 500;

/// The deployable that asks.
const PRODUCER: &str = "ratatoskr-chatgpt";

/// Why an erasure command could not be built or queued.
#[derive(Debug, thiserror::Error)]
pub(crate) enum EnqueueError {
    /// An owner, a sequence or an identity does not satisfy the command contract.
    #[error("a stream erasure command could not be constructed")]
    Invalid,
    /// The outbox could not be written.
    #[error("the stream erasure command could not be queued")]
    Database(#[from] sqlx::Error),
    /// The command envelope could not be encoded.
    #[error("the stream erasure command could not be encoded")]
    Encode(#[from] serde_json::Error),
}

/// Everything one owner's erasure commands need.
#[derive(Debug)]
pub(crate) struct OwnerErasure<'a> {
    /// The envelope tenant of the facts, `user:<uuid>`.
    pub(crate) owner: &'a str,
    /// The stream sequences to erase; a set is ascending and unique by construction.
    pub(crate) sequences: &'a BTreeSet<i64>,
    /// The identity the command names as `privacy_request:<id>` and stores as the row aggregate.
    /// A deletion passes its request id; the pump passes the event id of the orphaned row, which
    /// names no real deletion request.
    pub(crate) request_id: Uuid,
    /// The archive account the outbox row is bound to.
    pub(crate) account_id: Option<Uuid>,
    /// The correlation id of the issuing request, if any.
    pub(crate) correlation_id: Option<Uuid>,
    /// When the command is issued.
    pub(crate) issued_at: WireTimestamp,
    /// The deduplication key without the chunk index, so a second attempt enqueues nothing more.
    pub(crate) key_stem: &'a str,
}

/// Enqueues the erasure commands of one owner, in the caller's transaction: at most 500 ascending
/// sequences each, one outbox row per command, deduplicated by `key_stem` and the chunk index.
pub(crate) async fn enqueue_owner_erasure(
    transaction: &mut Transaction<'_, Postgres>,
    erasure: &OwnerErasure<'_>,
) -> Result<(), EnqueueError> {
    let request_ref = EntityRef::new(
        EntityKind::parse("privacy_request").map_err(|_| EnqueueError::Invalid)?,
        EntityLocalId::parse(&erasure.request_id.to_string()).map_err(|_| EnqueueError::Invalid)?,
    );
    let producer = ProducerName::parse(PRODUCER).map_err(|_| EnqueueError::Invalid)?;
    let owner_ref = TenantRef::parse(erasure.owner).map_err(|_| EnqueueError::Invalid)?;
    let sequences: Vec<u64> = erasure
        .sequences
        .iter()
        .map(|sequence| u64::try_from(*sequence))
        .collect::<Result<_, _>>()
        .map_err(|_| EnqueueError::Invalid)?;
    for (index, chunk) in sequences.chunks(MAX_SEQUENCES_PER_COMMAND).enumerate() {
        let command_id = CommandId::new_v7();
        let mut envelope = CommandEnvelope {
            command_id,
            command_type: PlatformStreamErasureRequested::command_type(),
            issued_at: erasure.issued_at,
            producer: producer.clone(),
            aggregate_id: request_ref.clone(),
            correlation_id: erasure.correlation_id.map_or_else(
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
            .map_err(|_| EnqueueError::Invalid)?;
        sqlx::query(
            "INSERT INTO chatgpt_archive.outbox_events
             (event_type, aggregate_id, tenant_id, export_id, payload, correlation_id,
              deduplication_key)
             VALUES ($1, $2, $3, NULL, $4, $5, $6)
             ON CONFLICT (deduplication_key) WHERE deduplication_key IS NOT NULL DO NOTHING",
        )
        .bind(PlatformStreamErasureRequested::COMMAND_TYPE)
        .bind(erasure.request_id)
        .bind(erasure.account_id)
        .bind(serde_json::to_value(&envelope)?)
        .bind(envelope.correlation_id.as_uuid())
        .bind(format!("{}:{index}", erasure.key_stem))
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}
