//! Deterministic digests for normalized parser evidence.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};
use sha2::Digest as _;

use crate::{ContentPartKind, MessageRole, ParsedConversation, ParsedMessage};

pub(super) fn conversation_digest(conversation: &ParsedConversation) -> String {
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
