//! Transactional publication records for normalized AI archive facts.
//!
//! Every row stores the complete [`EventEnvelope`] the bus will carry, never a bare
//! payload (XR-021 CONTRACTS.md section S02), so the publisher relays the stored
//! document unchanged and a retry can never mint a second payload.

use ratatoskr_ai_archive_contracts::{
    AiArchiveImport, AiArchiveTombstone, AiArchiveTombstoneSubject, AiConversationAdded,
    AiConversationUpdated, AiProjectAdded, AiProjectUpdated,
};
use ratatoskr_event_envelope::{EnvelopeSchemaVersion, EventEnvelope, EventPayload, ProducerName};
use ratatoskr_identifiers::{
    EntityKind, EntityLocalId, EntityRef, EventId, Extensions, TenantRef, WireTimestamp,
};
use uuid::Uuid;

use crate::Database;

/// The deployable that asserts every archive fact.
const PRODUCER: &str = "ratatoskr-chatgpt";

/// One validated normalized event ready for the archive-owned transactional outbox.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedArchiveEvent {
    /// Stable routing subject from the published contract.
    pub event_type: &'static str,
    /// Owning normalized aggregate identity, the row's `aggregate_id`.
    pub aggregate_id: Uuid,
    /// The complete envelope the bus carries.
    pub envelope: EventEnvelope,
}

/// Where an outbox row belongs, beyond the event itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutboxPlacement {
    /// The archive account the row belongs to, when it is tenant data.
    pub account_id: Option<Uuid>,
    /// The raw export the row depends on. A row bound to an export is removed with it.
    pub export_id: Option<Uuid>,
    /// Makes the insert idempotent: a second row with the same key is not stored.
    pub deduplication_key: Option<String>,
}

/// Event construction or persistence failure without payload disclosure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OutboxError {
    /// A state-carried payload contradicted the provenance it carries.
    #[error("the normalized archive event contradicts its import provenance")]
    InvalidProvenance,
    /// A payload could not be encoded for durable outbox storage.
    #[error("the normalized archive event could not be encoded")]
    Encode(#[source] serde_json::Error),
    /// The archive outbox could not durably store the event.
    #[error("the normalized archive event could not be stored")]
    Store(#[source] sqlx::Error),
    /// The envelope could not be built from the payload.
    #[error("the normalized archive event envelope could not be built")]
    Envelope,
}

impl NormalizedArchiveEvent {
    /// Constructs a conforming completed-import event.
    ///
    /// # Errors
    ///
    /// Returns [`OutboxError`] if the envelope cannot be built.
    pub fn archive_imported(payload: &AiArchiveImport) -> Result<Self, OutboxError> {
        Self::encode(
            payload.ai_archive_id.0,
            payload.ai_archive_id.as_entity_ref(),
            payload.owner,
            payload.imported_at,
            payload,
        )
    }

    /// Constructs a conforming conversation-added event.
    ///
    /// # Errors
    ///
    /// Returns [`OutboxError::InvalidProvenance`] if the contract payload is inconsistent.
    pub fn conversation_added(payload: &AiConversationAdded) -> Result<Self, OutboxError> {
        payload
            .validate()
            .map_err(|_| OutboxError::InvalidProvenance)?;
        let id = payload.conversation.ai_conversation_id;
        Self::encode(
            id.0,
            id.as_entity_ref(),
            payload.conversation.owner,
            payload.import_provenance.imported_at,
            payload,
        )
    }

    /// Constructs a conforming conversation-updated event.
    ///
    /// # Errors
    ///
    /// Returns [`OutboxError::InvalidProvenance`] if the contract payload is inconsistent.
    pub fn conversation_updated(payload: &AiConversationUpdated) -> Result<Self, OutboxError> {
        payload
            .validate()
            .map_err(|_| OutboxError::InvalidProvenance)?;
        let id = payload.conversation.ai_conversation_id;
        Self::encode(
            id.0,
            id.as_entity_ref(),
            payload.conversation.owner,
            payload.import_provenance.imported_at,
            payload,
        )
    }

    /// Constructs a conforming project-added event.
    ///
    /// # Errors
    ///
    /// Returns [`OutboxError::InvalidProvenance`] if the contract payload is inconsistent.
    pub fn project_added(payload: &AiProjectAdded) -> Result<Self, OutboxError> {
        payload
            .validate()
            .map_err(|_| OutboxError::InvalidProvenance)?;
        let id = payload.project.ai_project_id;
        Self::encode(
            id.0,
            id.as_entity_ref(),
            payload.import_provenance.owner,
            payload.import_provenance.imported_at,
            payload,
        )
    }

    /// Constructs a conforming project-updated event.
    ///
    /// # Errors
    ///
    /// Returns [`OutboxError::InvalidProvenance`] if the contract payload is inconsistent.
    pub fn project_updated(payload: &AiProjectUpdated) -> Result<Self, OutboxError> {
        payload
            .validate()
            .map_err(|_| OutboxError::InvalidProvenance)?;
        let id = payload.project.ai_project_id;
        Self::encode(
            id.0,
            id.as_entity_ref(),
            payload.import_provenance.owner,
            payload.import_provenance.imported_at,
            payload,
        )
    }

    /// Constructs an explicit-deletion tombstone event.
    ///
    /// The envelope names the tombstoned subject as its aggregate and the payload
    /// owner as its tenant; `occurred_at` is the instant the deletion was observed.
    ///
    /// # Errors
    ///
    /// Returns [`OutboxError`] if the envelope cannot be built.
    pub fn tombstoned(payload: &AiArchiveTombstone) -> Result<Self, OutboxError> {
        Self::encode(
            payload.ai_archive_id.0,
            tombstone_subject_ref(payload)?,
            payload.owner,
            payload.observed_at,
            payload,
        )
    }

    /// Correlates the event with the unit of work that caused it instead of itself.
    #[must_use]
    pub fn with_correlation(mut self, correlation: EntityRef) -> Self {
        self.envelope.correlation_id = correlation;
        self
    }

    fn encode<T: EventPayload>(
        aggregate_id: Uuid,
        aggregate_ref: EntityRef,
        tenant: TenantRef,
        occurred_at: WireTimestamp,
        payload: &T,
    ) -> Result<Self, OutboxError> {
        let event_id = EventId::new_v7();
        let serde_json::Value::Object(body) =
            serde_json::to_value(payload).map_err(OutboxError::Encode)?
        else {
            return Err(OutboxError::Envelope);
        };
        let envelope = EventEnvelope {
            event_id,
            event_type: T::event_type(),
            occurred_at,
            producer: ProducerName::parse(PRODUCER).map_err(|_| OutboxError::Envelope)?,
            aggregate_id: aggregate_ref,
            // A root event correlates to itself until the caller names its cause.
            correlation_id: event_id.as_entity_ref(),
            causation_id: None,
            tenant_id: Some(tenant),
            schema_version: EnvelopeSchemaVersion::CURRENT,
            payload: body,
            extensions: Extensions::new(),
        };
        Ok(Self {
            event_type: T::EVENT_TYPE,
            aggregate_id,
            envelope,
        })
    }
}

/// The aggregate a tombstone is about: the archive, or one conversation, project or
/// artifact inside it.
fn tombstone_subject_ref(payload: &AiArchiveTombstone) -> Result<EntityRef, OutboxError> {
    Ok(match &payload.subject {
        AiArchiveTombstoneSubject::Archive => payload.ai_archive_id.as_entity_ref(),
        AiArchiveTombstoneSubject::Conversation { ai_conversation_id } => {
            ai_conversation_id.as_entity_ref()
        }
        AiArchiveTombstoneSubject::Project { ai_project_id } => ai_project_id.as_entity_ref(),
        AiArchiveTombstoneSubject::Artifact {
            external_artifact_id,
        } => EntityRef::new(
            EntityKind::parse("artifact").map_err(|_| OutboxError::Envelope)?,
            EntityLocalId::parse(external_artifact_id.as_str())
                .map_err(|_| OutboxError::Envelope)?,
        ),
    })
}

impl Database {
    /// Durably appends a validated event in the transaction that owns the normalized
    /// mutation, and reports whether a row was added.
    ///
    /// A second event with the same deduplication key is not stored and yields
    /// `false`.
    ///
    /// # Errors
    ///
    /// Returns [`OutboxError`] when the event cannot be stored.
    pub async fn enqueue_normalized_event(
        transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        event: &NormalizedArchiveEvent,
        placement: &OutboxPlacement,
    ) -> Result<bool, OutboxError> {
        let payload = serde_json::to_value(&event.envelope).map_err(OutboxError::Encode)?;
        let inserted = sqlx::query(
            "INSERT INTO chatgpt_archive.outbox_events
             (event_type, aggregate_id, tenant_id, export_id, payload, correlation_id,
              deduplication_key)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (deduplication_key) WHERE deduplication_key IS NOT NULL DO NOTHING",
        )
        .bind(event.event_type)
        .bind(event.aggregate_id)
        .bind(placement.account_id)
        .bind(placement.export_id)
        .bind(payload)
        .bind(event.envelope.correlation_id.as_uuid())
        .bind(&placement.deduplication_key)
        .execute(&mut **transaction)
        .await
        .map_err(OutboxError::Store)?;
        Ok(inserted.rows_affected() == 1)
    }
}
