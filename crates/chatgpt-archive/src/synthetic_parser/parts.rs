//! Part classification and lossless raw preservation helpers.

use serde_json::{Map, Value};

use super::{ContentPartKind, MessageRole, ParsedContentPart, RawRecord};

pub(super) fn parse_parts(
    values: Vec<Value>,
    content_path: &str,
    raw_records: &mut Vec<RawRecord>,
) -> Vec<ParsedContentPart> {
    values
        .into_iter()
        .enumerate()
        .map(|(ordinal, payload)| {
            let kind = part_kind(&payload);
            if kind == ContentPartKind::Unknown {
                raw_records.push(RawRecord {
                    path: child_path(&child_path(content_path, "parts"), &ordinal.to_string()),
                    payload: payload.clone(),
                });
            }
            ParsedContentPart {
                ordinal,
                kind,
                payload,
            }
        })
        .collect()
}

pub(super) fn normalize_role(role: &str) -> MessageRole {
    match role {
        "system" => MessageRole::System,
        "user" => MessageRole::User,
        "assistant" => MessageRole::Assistant,
        "tool" => MessageRole::Tool,
        "internal" => MessageRole::Internal,
        _ => MessageRole::Unknown,
    }
}

pub(super) fn record_extra(
    raw_records: &mut Vec<RawRecord>,
    path: &str,
    values: &Map<String, Value>,
) {
    raw_records.extend(values.iter().map(|(key, payload)| RawRecord {
        path: child_path(path, key),
        payload: payload.clone(),
    }));
}

pub(super) fn child_path(path: &str, segment: &str) -> String {
    let escaped = segment.replace('~', "~0").replace('/', "~1");
    format!("{path}/{escaped}")
}

fn part_kind(payload: &Value) -> ContentPartKind {
    if payload.is_string() {
        return ContentPartKind::Text;
    }
    let Some(object) = payload.as_object() else {
        return ContentPartKind::Unknown;
    };
    match object.get("kind").and_then(Value::as_str) {
        Some("tool_call") => ContentPartKind::ToolCall,
        Some("tool_result") => ContentPartKind::ToolResult,
        Some("media_reference") => media_kind(object),
        _ => ContentPartKind::Unknown,
    }
}

fn media_kind(object: &Map<String, Value>) -> ContentPartKind {
    if object
        .get("mime_type")
        .and_then(Value::as_str)
        .is_some_and(|media_type| media_type.starts_with("image/"))
    {
        ContentPartKind::Image
    } else {
        ContentPartKind::File
    }
}
