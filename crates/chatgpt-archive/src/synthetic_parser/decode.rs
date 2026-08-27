//! Typed decoding and raw-field preservation for the synthetic shape.

use serde_json::Value;

use super::input::{ConversationInput, MappingInput, MessageInput};
use super::parts::{child_path, normalize_role, parse_parts, record_extra};
use super::{
    MessageRole, ParsedConversation, ParsedConversations, ParsedMessage, RawRecord,
    SYNTHETIC_SCHEMA_ID, SyntheticParserError,
};
use crate::ParserId;

pub(super) fn parse(
    source: &[u8],
    parser: ParserId,
) -> Result<ParsedConversations, SyntheticParserError> {
    let inputs: Vec<ConversationInput> =
        serde_json::from_slice(source).map_err(|_| SyntheticParserError::InvalidDocument)?;
    let mut raw_records = Vec::new();
    let conversations = inputs
        .into_iter()
        .enumerate()
        .map(|(index, input)| parse_conversation(input, index, &mut raw_records))
        .collect();
    Ok(ParsedConversations {
        schema_id: SYNTHETIC_SCHEMA_ID.to_owned(),
        parser,
        conversations,
        raw_records,
    })
}

fn parse_conversation(
    input: ConversationInput,
    index: usize,
    raw_records: &mut Vec<RawRecord>,
) -> ParsedConversation {
    let path = format!("/{index}");
    record_extra(raw_records, &path, &input.extra);
    let messages = input
        .mapping
        .into_iter()
        .filter_map(|(key, item)| {
            parse_mapping(item, &child_path(&path, "mapping"), &key, raw_records)
        })
        .collect();
    ParsedConversation {
        external_id: input.id,
        title: input.title,
        created_at_epoch_seconds: input.create_time,
        updated_at_epoch_seconds: input.update_time,
        provider_metadata: Value::Object(input.extra),
        messages,
    }
}

fn parse_mapping(
    input: MappingInput,
    mapping_path: &str,
    key: &str,
    raw_records: &mut Vec<RawRecord>,
) -> Option<ParsedMessage> {
    let path = child_path(mapping_path, key);
    record_extra(raw_records, &path, &input.extra);
    if let Some(id) = input.id {
        raw_records.push(RawRecord {
            path: child_path(&path, "id"),
            payload: Value::String(id),
        });
    }
    input
        .message
        .map(|message| parse_message(message, input.parent, &path, raw_records))
}

fn parse_message(
    input: MessageInput,
    parent_external_id: Option<String>,
    mapping_path: &str,
    raw_records: &mut Vec<RawRecord>,
) -> ParsedMessage {
    let path = child_path(mapping_path, "message");
    record_extra(raw_records, &path, &input.extra);
    record_extra(
        raw_records,
        &child_path(&path, "author"),
        &input.author.extra,
    );
    record_extra(
        raw_records,
        &child_path(&path, "metadata"),
        &input.metadata.extra,
    );
    record_extra(
        raw_records,
        &child_path(&path, "content"),
        &input.content.extra,
    );
    let role = normalize_role(&input.author.role);
    if role == MessageRole::Unknown {
        raw_records.push(RawRecord {
            path: child_path(&child_path(&path, "author"), "role"),
            payload: Value::String(input.author.role.clone()),
        });
    }
    ParsedMessage {
        external_id: input.id,
        parent_external_id,
        role,
        created_at_epoch_seconds: input.create_time,
        updated_at_epoch_seconds: input.update_time,
        model_slug: input.metadata.model_slug,
        provider_metadata: Value::Object(input.extra),
        parts: parse_parts(
            input.content.parts,
            &child_path(&path, "content"),
            raw_records,
        ),
    }
}
