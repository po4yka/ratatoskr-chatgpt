//! Helpers for the tests of the tombstone acknowledgement (XR-021 CONTRACTS.md R5-11).

use ratatoskr_event_envelope::{
    EnvelopeSchemaVersion, EventEnvelope, EventPayload as _, ProducerName,
};
use ratatoskr_identifiers::{EventId, Extensions, WireTimestamp};
use ratatoskr_operation_contracts::KnowledgeDeletionAcknowledged;

/// A valid acknowledgement of `event_id` for `owner`: `applied`, or `refused` as a foreign subject.
pub(crate) fn acknowledgement(
    event_id: EventId,
    owner: &str,
    outcome: &str,
) -> KnowledgeDeletionAcknowledged {
    let refusal = (outcome == "refused").then_some("foreign_subject");
    serde_json::from_value(serde_json::json!({
        "acknowledged_event_id": event_id.to_string(),
        "family": "ai_archive",
        "owner": owner,
        "subject": "ai_archive:018f0000-0000-7000-8000-000000000402",
        "outcome": outcome,
        "refusal": refusal,
        "acknowledged_at": "2026-10-12T10:00:00Z",
    }))
    .expect("a literal acknowledgement satisfies the contract")
}

/// The envelope Knowledge publishes for `acknowledgement`.
pub(crate) fn acknowledgement_envelope(
    acknowledgement: &KnowledgeDeletionAcknowledged,
) -> EventEnvelope {
    let serde_json::Value::Object(payload) =
        serde_json::to_value(acknowledgement).expect("the acknowledgement encodes")
    else {
        unreachable!("an acknowledgement is a JSON object");
    };
    let event_id = EventId::new_v7();
    EventEnvelope {
        event_id,
        event_type: KnowledgeDeletionAcknowledged::event_type(),
        occurred_at: WireTimestamp::now(),
        producer: ProducerName::parse("ratatoskr-knowledge").expect("a literal producer"),
        aggregate_id: acknowledgement.subject.clone(),
        correlation_id: event_id.as_entity_ref(),
        causation_id: Some(acknowledgement.acknowledged_event_id.as_entity_ref()),
        tenant_id: Some(acknowledgement.owner),
        schema_version: EnvelopeSchemaVersion::CURRENT,
        payload,
        extensions: Extensions::new(),
    }
}

/// The acknowledgement state of the outbox row whose envelope carries `event_id`.
#[derive(Debug)]
pub(crate) struct RowState {
    pub(crate) published: bool,
    pub(crate) acknowledged: bool,
    pub(crate) ack_outcome: Option<String>,
    pub(crate) resend_count: i32,
    pub(crate) attempt_count: i32,
    /// Seconds from now until the next resend, when one is scheduled.
    pub(crate) seconds_to_resend: Option<f64>,
}

pub(crate) async fn row_state(pool: &sqlx::PgPool, event_id: EventId) -> RowState {
    let (published, acknowledged, ack_outcome, resend_count, attempt_count, seconds_to_resend): (
        bool,
        bool,
        Option<String>,
        i32,
        i32,
        Option<f64>,
    ) = sqlx::query_as(
        "SELECT published_at IS NOT NULL, acknowledged_at IS NOT NULL, ack_outcome, resend_count,
                attempt_count,
                EXTRACT(EPOCH FROM (next_resend_at - now()))::float8
         FROM chatgpt_archive.outbox_events WHERE payload ->> 'event_id' = $1",
    )
    .bind(event_id.to_string())
    .fetch_one(pool)
    .await
    .expect("the tombstone row is readable");
    RowState {
        published,
        acknowledged,
        ack_outcome,
        resend_count,
        attempt_count,
        seconds_to_resend,
    }
}
