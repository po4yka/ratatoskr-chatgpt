//! The closed table from outbox event types to subjects, and the stored document as the bytes a
//! publish carries.

use ratatoskr_event_envelope::{CommandEnvelope, EventEnvelope};

use super::PublishError;

/// Subject Platform consumes to project producer operation facts.
const OPERATION_REPORTED_SUBJECT: &str = "evt.ai-archive.chatgpt.operation.reported.v1";
/// The one command this service produces: a request to Edge to erase stream messages.
const STREAM_ERASURE_COMMAND: &str = "platform.stream_erasure.requested.v1";

/// The subject an outbox `event_type` publishes to, or none when the type is not one
/// this service produces.
///
/// The operation report keeps its per-provider ingress subject; the eight archive
/// facts publish to `evt.` plus their contract type name. The match is closed on
/// purpose: a type outside it is a programming error, never a row to skip.
pub(super) fn subject_for_event_type(event_type: &str) -> Option<&'static str> {
    Some(match event_type {
        "platform.operation.reported.v1" => OPERATION_REPORTED_SUBJECT,
        "ai_archive.archive.imported.v1" => "evt.ai_archive.archive.imported.v1",
        "ai_archive.conversation.added.v1" => "evt.ai_archive.conversation.added.v1",
        "ai_archive.conversation.updated.v1" => "evt.ai_archive.conversation.updated.v1",
        "ai_archive.project.added.v1" => "evt.ai_archive.project.added.v1",
        "ai_archive.project.updated.v1" => "evt.ai_archive.project.updated.v1",
        "ai_archive.artifact.added.v1" => "evt.ai_archive.artifact.added.v1",
        "ai_archive.artifact.updated.v1" => "evt.ai_archive.artifact.updated.v1",
        "ai_archive.subject.tombstoned.v1" => "evt.ai_archive.subject.tombstoned.v1",
        // A command publishes to `cmd.` plus its command type (XR-021 CONTRACTS.md S02 rule 2).
        "platform.stream_erasure.requested.v1" => "cmd.platform.stream_erasure.requested.v1",
        _ => return None,
    })
}

/// The stored document as the bytes to publish and its event or command id, after proving it is
/// the envelope of the row's declared type.
pub(super) fn stored_envelope(
    event_type: &str,
    payload: &serde_json::Value,
) -> Result<(Vec<u8>, String), PublishError> {
    let bytes = serde_json::to_vec(payload).map_err(PublishError::Encode)?;
    if event_type == STREAM_ERASURE_COMMAND {
        let command = CommandEnvelope::from_json(&bytes).map_err(|_| PublishError::InvalidRow)?;
        if command.command_type.to_wire() != event_type {
            return Err(PublishError::InvalidRow);
        }
        return Ok((bytes, command.command_id.to_string()));
    }
    let envelope = EventEnvelope::from_json(&bytes).map_err(|_| PublishError::InvalidRow)?;
    if envelope.event_type.to_wire() != event_type {
        return Err(PublishError::InvalidRow);
    }
    Ok((bytes, envelope.event_id.to_string()))
}

pub(super) async fn publish_row(
    jetstream: &async_nats::jetstream::Context,
    subject: &'static str,
    event_id: &str,
    body: Vec<u8>,
) -> Result<u64, PublishError> {
    let mut headers = async_nats::HeaderMap::new();
    headers.insert("Nats-Msg-Id", event_id);
    let acknowledgement = jetstream
        .publish_with_headers(subject, headers, body.into())
        .await
        .map_err(PublishError::broker)?;
    // A duplicate acknowledgement carries the sequence of the message the broker already held.
    let acknowledgement = acknowledgement.await.map_err(PublishError::broker)?;
    Ok(acknowledgement.sequence)
}

#[cfg(test)]
mod tests {
    use super::subject_for_event_type;

    #[test]
    fn operation_reports_keep_their_per_provider_subject() {
        assert_eq!(
            subject_for_event_type("platform.operation.reported.v1"),
            Some("evt.ai-archive.chatgpt.operation.reported.v1")
        );
    }

    #[test]
    fn the_eight_archive_facts_publish_to_evt_plus_their_type() {
        for event_type in [
            "ai_archive.archive.imported.v1",
            "ai_archive.conversation.added.v1",
            "ai_archive.conversation.updated.v1",
            "ai_archive.project.added.v1",
            "ai_archive.project.updated.v1",
            "ai_archive.artifact.added.v1",
            "ai_archive.artifact.updated.v1",
            "ai_archive.subject.tombstoned.v1",
        ] {
            let expected = format!("evt.{event_type}");
            assert_eq!(
                subject_for_event_type(event_type),
                Some(expected.as_str()),
                "{event_type}"
            );
        }
    }

    #[test]
    fn the_stream_erasure_command_publishes_to_cmd_plus_its_type() {
        assert_eq!(
            subject_for_event_type("platform.stream_erasure.requested.v1"),
            Some("cmd.platform.stream_erasure.requested.v1")
        );
    }

    #[test]
    fn any_other_type_has_no_subject() {
        assert_eq!(subject_for_event_type("social.source.captured.v1"), None);
        assert_eq!(subject_for_event_type(""), None);
    }
}
