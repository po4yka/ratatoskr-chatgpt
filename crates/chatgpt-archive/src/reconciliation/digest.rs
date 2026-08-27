//! Deterministic digests for normalized parser evidence.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};
use sha2::Digest as _;

use crate::{
    AssetAvailability, AssetKind, ContentPartKind, InstructionKind, MessageRole, ParsedAsset,
    ParsedCanvasDocument, ParsedConversation, ParsedInstruction, ParsedMessage, ParsedProject,
};

pub(crate) fn conversation_digest(conversation: &ParsedConversation) -> String {
    let mut messages = conversation.messages.iter().collect::<Vec<_>>();
    messages.sort_by(|left, right| left.external_id.cmp(&right.external_id));
    digest(&json!({
        "external_id": conversation.external_id,
        "title": conversation.title,
        "created_at_epoch_seconds": conversation.created_at_epoch_seconds,
        "updated_at_epoch_seconds": conversation.updated_at_epoch_seconds,
        "provider_metadata": canonicalize(&conversation.provider_metadata),
        "messages": messages.into_iter().map(message_value).collect::<Vec<_>>(),
    }))
}

pub(super) fn message_digest(message: &ParsedMessage) -> String {
    digest(&message_value(message))
}

pub(super) fn project_digest(project: &ParsedProject) -> String {
    digest(&json!({
        "external_id": project.external_id,
        "title": project.title,
        "description": project.description,
        "conversation_external_ids": project.conversation_external_ids,
        "asset_external_ids": project.asset_external_ids,
        "provider_metadata": canonicalize(&project.provider_metadata),
    }))
}

pub(super) fn instruction_digest(instruction: &ParsedInstruction) -> String {
    digest(&json!({
        "external_id": instruction.external_id,
        "ordinal": instruction.ordinal,
        "kind": instruction_kind_name(instruction.kind),
        "content": canonicalize(&instruction.content),
        "provider_metadata": canonicalize(&instruction.provider_metadata),
    }))
}

pub(super) fn canvas_digest(document: &ParsedCanvasDocument) -> String {
    digest(&json!({
        "external_id": document.external_id,
        "project_external_id": document.project_external_id,
        "conversation_external_id": document.conversation_external_id,
        "content": document.content.iter().map(canonicalize).collect::<Vec<_>>(),
        "provider_metadata": canonicalize(&document.provider_metadata),
    }))
}

pub(super) fn asset_digest(asset: &ParsedAsset) -> String {
    digest(&json!({
        "external_id": asset.external_id,
        "kind": asset_kind_name(asset.kind),
        "project_external_id": asset.project_external_id,
        "conversation_external_id": asset.conversation_external_id,
        "display_name": asset.display_name,
        "media_type": asset.media_type,
        "availability": availability_name(asset.availability),
        "blob": asset.blob,
        "anomaly": asset.anomaly.map(anomaly_name),
        "provider_metadata": canonicalize(&asset.provider_metadata),
    }))
}

fn digest(value: &Value) -> String {
    let bytes = canonicalize(value).to_string();
    hex::encode(sha2::Sha256::digest(bytes.as_bytes()))
}

fn message_value(message: &ParsedMessage) -> Value {
    json!({
        "external_id": message.external_id,
        "parent_external_id": message.parent_external_id,
        "role": role_name(&message.role),
        "created_at_epoch_seconds": message.created_at_epoch_seconds,
        "updated_at_epoch_seconds": message.updated_at_epoch_seconds,
        "model_slug": message.model_slug,
        "provider_metadata": canonicalize(&message.provider_metadata),
        "parts": message.parts.iter().map(|part| json!({
            "ordinal": part.ordinal,
            "kind": part_kind_name(&part.kind),
            "payload": canonicalize(&part.payload),
        })).collect::<Vec<_>>(),
    })
}

fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonicalize).collect()),
        Value::Object(values) => {
            let sorted = values
                .iter()
                .map(|(key, value)| (key.clone(), canonicalize(value)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(Map::from_iter(sorted))
        }
        scalar => scalar.clone(),
    }
}

fn role_name(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
        MessageRole::Internal => "internal",
        MessageRole::Unknown => "unknown",
    }
}

fn part_kind_name(kind: &ContentPartKind) -> &'static str {
    match kind {
        ContentPartKind::Text => "text",
        ContentPartKind::ToolCall => "tool_call",
        ContentPartKind::ToolResult => "tool_result",
        ContentPartKind::Image => "image",
        ContentPartKind::File => "file",
        ContentPartKind::Unknown => "unknown",
    }
}

fn instruction_kind_name(kind: InstructionKind) -> &'static str {
    match kind {
        InstructionKind::Instruction => "instruction",
        InstructionKind::SystemPrompt => "system_prompt",
        InstructionKind::Unknown => "unknown",
    }
}

fn asset_kind_name(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Uploaded => "uploaded",
        AssetKind::Generated => "generated",
        AssetKind::Unknown => "unknown",
    }
}

fn availability_name(availability: AssetAvailability) -> &'static str {
    match availability {
        AssetAvailability::Missing => "missing",
        AssetAvailability::Quarantined => "quarantined",
        AssetAvailability::Verified => "verified",
    }
}

fn anomaly_name(anomaly: crate::AssetAnomaly) -> &'static str {
    match anomaly {
        crate::AssetAnomaly::MissingArtifact => "missing_artifact",
        crate::AssetAnomaly::ExtractedArtifactQuarantined => "extracted_artifact_quarantined",
        crate::AssetAnomaly::BlobVerificationFailed => "blob_verification_failed",
        crate::AssetAnomaly::DigestMismatch => "digest_mismatch",
        crate::AssetAnomaly::LengthMismatch => "length_mismatch",
        crate::AssetAnomaly::MediaTypeMismatch => "media_type_mismatch",
        crate::AssetAnomaly::InvalidDeclaration => "invalid_declaration",
        crate::AssetAnomaly::AmbiguousArtifact => "ambiguous_artifact",
    }
}
