//! Transactional publication records for normalized AI archive facts.

use ratatoskr_ai_archive_contracts::{
    AiArchiveTombstone, AiConversationAdded, AiConversationUpdated, AiProjectAdded,
    AiProjectUpdated,
};
use ratatoskr_event_envelope::EventPayload;
use uuid::Uuid;

use crate::Database;

/// One validated normalized event ready for the archive-owned transactional outbox.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedArchiveEvent {
    /// Stable routing subject from the published contract.
    pub event_type: &'static str,
    /// Owning normalized aggregate identity.
    pub aggregate_id: Uuid,
    /// Complete state-carried contract payload.
    pub payload: serde_json::Value,
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
}

impl NormalizedArchiveEvent {
    /// Constructs a conforming conversation-added event.
    ///
    /// # Errors
    ///
    /// Returns [`OutboxError::InvalidProvenance`] if the contract payload is inconsistent.
    pub fn conversation_added(payload: AiConversationAdded) -> Result<Self, OutboxError> {
        payload
            .validate()
            .map_err(|_| OutboxError::InvalidProvenance)?;
        Self::encode(payload.conversation.ai_conversation_id.0, payload)
    }

    /// Constructs a conforming conversation-updated event.
    ///
    /// # Errors
    ///
    /// Returns [`OutboxError::InvalidProvenance`] if the contract payload is inconsistent.
    pub fn conversation_updated(payload: AiConversationUpdated) -> Result<Self, OutboxError> {
        payload
            .validate()
            .map_err(|_| OutboxError::InvalidProvenance)?;
        Self::encode(payload.conversation.ai_conversation_id.0, payload)
    }

    /// Constructs a conforming project-added event.
    ///
    /// # Errors
    ///
    /// Returns [`OutboxError::InvalidProvenance`] if the contract payload is inconsistent.
    pub fn project_added(payload: AiProjectAdded) -> Result<Self, OutboxError> {
        payload
            .validate()
            .map_err(|_| OutboxError::InvalidProvenance)?;
        Self::encode(payload.project.ai_project_id.0, payload)
    }

    /// Constructs a conforming project-updated event.
    ///
    /// # Errors
    ///
    /// Returns [`OutboxError::InvalidProvenance`] if the contract payload is inconsistent.
    pub fn project_updated(payload: AiProjectUpdated) -> Result<Self, OutboxError> {
        payload
            .validate()
            .map_err(|_| OutboxError::InvalidProvenance)?;
        Self::encode(payload.project.ai_project_id.0, payload)
    }

    /// Constructs an explicit-deletion tombstone event.
    ///
    /// # Errors
    ///
    /// Returns [`OutboxError`] if the payload cannot be encoded.
    pub fn tombstoned(payload: AiArchiveTombstone) -> Result<Self, OutboxError> {
        Self::encode(payload.ai_archive_id.0, payload)
    }

    fn encode<T: EventPayload + serde::Serialize>(
        aggregate_id: Uuid,
        payload: T,
    ) -> Result<Self, OutboxError> {
        Ok(Self {
            event_type: T::EVENT_TYPE,
            aggregate_id,
            payload: serde_json::to_value(payload).map_err(OutboxError::Encode)?,
        })
    }
}

impl Database {
    /// Durably appends a validated event in the transaction that owns the normalized mutation.
    ///
    /// # Errors
    ///
    /// Returns [`OutboxError`] when the event cannot be stored.
    pub async fn enqueue_normalized_event(
        &self,
        event: &NormalizedArchiveEvent,
        correlation_id: Option<Uuid>,
    ) -> Result<(), OutboxError> {
        sqlx::query(
            "insert into chatgpt_archive.outbox_events
             (event_type, aggregate_id, payload, correlation_id)
             values ($1, $2, $3, $4)",
        )
        .bind(event.event_type)
        .bind(event.aggregate_id)
        .bind(&event.payload)
        .bind(correlation_id)
        .execute(self.pool())
        .await
        .map_err(OutboxError::Store)?;
        Ok(())
    }
}
