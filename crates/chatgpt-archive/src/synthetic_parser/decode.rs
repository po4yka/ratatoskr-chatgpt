//! Typed decoding and raw-field preservation for the synthetic shape.

use serde_json::Value;

use super::input::{
    CanvasInput, ConversationInput, InstructionInput, MappingInput, MessageInput, ProjectInput,
};
use super::parts::{child_path, normalize_role, parse_parts, record_extra};
use super::{
    InstructionKind, MessageRole, ParsedCanvasDocument, ParsedConversation, ParsedConversations,
    ParsedInstruction, ParsedMessage, ParsedProject, RawRecord, SYNTHETIC_SCHEMA_ID,
    SyntheticParserError,
};
use crate::ParserId;

pub(super) fn parse_archive(
    conversations_source: &[u8],
    projects_source: Option<&[u8]>,
    canvas_source: Option<&[u8]>,
    parser: ParserId,
) -> Result<ParsedConversations, SyntheticParserError> {
    let inputs: Vec<ConversationInput> = serde_json::from_slice(conversations_source)
        .map_err(|_| SyntheticParserError::InvalidDocument)?;
    let mut raw_records = Vec::new();
    let conversations = inputs
        .into_iter()
        .enumerate()
        .map(|(index, input)| parse_conversation(input, index, &mut raw_records))
        .collect();
    let projects = parse_projects(projects_source, &mut raw_records)?;
    let canvas_documents = parse_canvas(canvas_source, &mut raw_records)?;
    Ok(ParsedConversations {
        schema_id: SYNTHETIC_SCHEMA_ID.to_owned(),
        parser,
        conversations,
        projects,
        canvas_documents,
        assets: Vec::new(),
        raw_records,
    })
}

fn parse_projects(
    source: Option<&[u8]>,
    raw_records: &mut Vec<RawRecord>,
) -> Result<Vec<ParsedProject>, SyntheticParserError> {
    let Some(source) = source else {
        return Ok(Vec::new());
    };
    let inputs: Vec<ProjectInput> =
        serde_json::from_slice(source).map_err(|_| SyntheticParserError::InvalidDocument)?;
    Ok(inputs
        .into_iter()
        .enumerate()
        .map(|(index, input)| parse_project(input, index, raw_records))
        .collect())
}

fn parse_project(
    input: ProjectInput,
    index: usize,
    raw_records: &mut Vec<RawRecord>,
) -> ParsedProject {
    let path = format!("/projects/{index}");
    record_extra(raw_records, &path, &input.extra);
    let instructions = input
        .instructions
        .into_iter()
        .enumerate()
        .map(|(ordinal, instruction)| parse_instruction(instruction, ordinal, &path, raw_records))
        .collect();
    ParsedProject {
        external_id: input.id,
        title: input.title,
        description: input.description,
        instructions,
        conversation_external_ids: input.conversation_ids,
        asset_external_ids: input.asset_ids,
        provider_metadata: Value::Object(input.extra),
    }
}

fn parse_instruction(
    input: InstructionInput,
    ordinal: usize,
    project_path: &str,
    raw_records: &mut Vec<RawRecord>,
) -> ParsedInstruction {
    let path = child_path(
        &child_path(project_path, "instructions"),
        &ordinal.to_string(),
    );
    record_extra(raw_records, &path, &input.extra);
    let kind = match input.kind.as_str() {
        "instruction" => InstructionKind::Instruction,
        "system_prompt" => InstructionKind::SystemPrompt,
        _ => {
            raw_records.push(RawRecord {
                path: child_path(&path, "kind"),
                payload: Value::String(input.kind),
            });
            InstructionKind::Unknown
        }
    };
    ParsedInstruction {
        external_id: input.id,
        ordinal,
        kind,
        content: input.content,
        provider_metadata: Value::Object(input.extra),
    }
}

fn parse_canvas(
    source: Option<&[u8]>,
    raw_records: &mut Vec<RawRecord>,
) -> Result<Vec<ParsedCanvasDocument>, SyntheticParserError> {
    let Some(source) = source else {
        return Ok(Vec::new());
    };
    let inputs: Vec<CanvasInput> =
        serde_json::from_slice(source).map_err(|_| SyntheticParserError::InvalidDocument)?;
    Ok(inputs
        .into_iter()
        .enumerate()
        .map(|(index, input)| {
            let path = format!("/canvas/{index}");
            record_extra(raw_records, &path, &input.extra);
            ParsedCanvasDocument {
                external_id: input.id,
                project_external_id: input.project_id,
                conversation_external_id: input.conversation_id,
                content: input.content,
                provider_metadata: Value::Object(input.extra),
            }
        })
        .collect())
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
