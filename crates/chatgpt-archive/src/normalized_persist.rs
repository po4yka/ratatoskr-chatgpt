//! Persists the normalized detail that contract facts are projected from.
//!
//! The initial import and an applied reparse share these writers, so the rows a
//! fact is built from are identical on both paths: projects, conversation
//! headers, messages with their provider order and times, and content-part
//! revisions. Nothing here reads the in-memory parse again after the rows are
//! written; `contract_projection` reads the rows back.

use std::collections::BTreeMap;

use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{
    ContentPartKind, MessageRole, ParsedContentPart, ParsedConversation, ParsedConversations,
    ParsedMessage, ParsedProject, ParserId,
};

type Tx<'a> = Transaction<'a, Postgres>;

/// Latest representable provider instant, 9999-12-31T23:59:59Z in epoch seconds.
const MAX_EPOCH_SECONDS: f64 = 253_402_300_799.0;

/// Provider epoch seconds that `PostgreSQL` can store, or none.
fn storable_epoch(seconds: Option<f64>) -> Option<f64> {
    seconds.filter(|value| value.is_finite() && (0.0..=MAX_EPOCH_SECONDS).contains(value))
}

/// Upserts every project the parse evidenced and returns provider id to row id.
///
/// A project's parser stamp moves to `parser` only when its content changed, so a
/// newer parser release that reads the same project does not make it look new.
///
/// # Errors
///
/// Returns the database error when a row cannot be written.
pub(crate) async fn persist_projects(
    transaction: &mut Tx<'_>,
    tenant_id: Uuid,
    export_id: Uuid,
    parsed: &ParsedConversations,
    parser: &ParserId,
) -> Result<BTreeMap<String, Uuid>, sqlx::Error> {
    let mut ids = BTreeMap::new();
    for project in &parsed.projects {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO chatgpt_archive.projects
             (id, account_id, external_id, title, description, instructions,
              first_seen_export, last_seen_export, parser_name, parser_version)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $7, $8, $9)
             ON CONFLICT (account_id, external_id) DO UPDATE
             SET parser_name = CASE WHEN (chatgpt_archive.projects.title,
                                          chatgpt_archive.projects.description,
                                          chatgpt_archive.projects.instructions)
                                     IS NOT DISTINCT FROM
                                     (EXCLUDED.title, EXCLUDED.description, EXCLUDED.instructions)
                                     AND chatgpt_archive.projects.parser_name IS NOT NULL
                                THEN chatgpt_archive.projects.parser_name
                                ELSE EXCLUDED.parser_name END,
                 parser_version = CASE WHEN (chatgpt_archive.projects.title,
                                             chatgpt_archive.projects.description,
                                             chatgpt_archive.projects.instructions)
                                        IS NOT DISTINCT FROM
                                        (EXCLUDED.title, EXCLUDED.description,
                                         EXCLUDED.instructions)
                                        AND chatgpt_archive.projects.parser_version IS NOT NULL
                                   THEN chatgpt_archive.projects.parser_version
                                   ELSE EXCLUDED.parser_version END,
                 title = EXCLUDED.title, description = EXCLUDED.description,
                 instructions = EXCLUDED.instructions,
                 first_seen_export = COALESCE(chatgpt_archive.projects.first_seen_export,
                                              EXCLUDED.first_seen_export),
                 last_seen_export = EXCLUDED.last_seen_export, updated_at = now()
             RETURNING id",
        )
        .bind(Uuid::now_v7())
        .bind(tenant_id)
        .bind(&project.external_id)
        .bind(&project.title)
        .bind(&project.description)
        .bind(instruction_text(project))
        .bind(export_id)
        .bind(&parser.name)
        .bind(&parser.version)
        .fetch_one(&mut **transaction)
        .await?;
        sqlx::query(
            "INSERT INTO chatgpt_archive.export_entity_observations
             (export_id, entity_kind, entity_id) VALUES ($1, 'project', $2)
             ON CONFLICT DO NOTHING",
        )
        .bind(export_id)
        .bind(id)
        .execute(&mut **transaction)
        .await?;
        ids.insert(project.external_id.clone(), id);
    }
    Ok(ids)
}

/// The project's instructions as one text: each instruction in provider order,
/// separated by a blank line. String content is kept verbatim and any other
/// provider shape is kept as its JSON text, so nothing is dropped.
fn instruction_text(project: &ParsedProject) -> Option<String> {
    let mut instructions: Vec<_> = project.instructions.iter().collect();
    instructions.sort_by_key(|instruction| instruction.ordinal);
    let texts: Vec<String> = instructions
        .into_iter()
        .map(|instruction| match &instruction.content {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        })
        .collect();
    (!texts.is_empty()).then(|| texts.join("\n\n"))
}

/// Writes one conversation's header and every message with its content parts, and
/// stamps the conversation with the parser that produced these rows.
///
/// `conversation_id` is the row the caller located or created. Messages are
/// upserted by provider id, a content-part revision is added only when the
/// parts changed, and every message is observed in `export_id`.
///
/// # Errors
///
/// Returns the database error when a row cannot be written.
pub(crate) async fn persist_conversation_detail(
    transaction: &mut Tx<'_>,
    export_id: Uuid,
    conversation_id: Uuid,
    project_id: Option<Uuid>,
    conversation: &ParsedConversation,
    parser: &ParserId,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE chatgpt_archive.conversations
         SET title = $2,
             project_id = COALESCE($3, project_id),
             provider_created_at = to_timestamp($4::double precision),
             provider_updated_at = to_timestamp($5::double precision),
             first_seen_export = COALESCE(first_seen_export, $6),
             last_seen_export = $6,
             parser_name = $7,
             parser_version = $8,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(conversation_id)
    .bind(&conversation.title)
    .bind(project_id)
    .bind(storable_epoch(conversation.created_at_epoch_seconds))
    .bind(storable_epoch(conversation.updated_at_epoch_seconds))
    .bind(export_id)
    .bind(&parser.name)
    .bind(&parser.version)
    .execute(&mut **transaction)
    .await?;
    let mut message_ids = BTreeMap::new();
    for (ordinal, message) in conversation.messages.iter().enumerate() {
        let message_id =
            persist_message(transaction, export_id, conversation_id, ordinal, message).await?;
        persist_parts(transaction, message_id, &message.parts).await?;
        message_ids.insert(message.external_id.clone(), message_id);
    }
    for message in &conversation.messages {
        if let (Some(message_id), Some(parent_id)) = (
            message_ids.get(&message.external_id),
            message
                .parent_external_id
                .as_ref()
                .and_then(|parent| message_ids.get(parent)),
        ) {
            sqlx::query("UPDATE chatgpt_archive.messages SET parent_message_id = $2 WHERE id = $1")
                .bind(message_id)
                .bind(parent_id)
                .execute(&mut **transaction)
                .await?;
        }
    }
    Ok(())
}

async fn persist_message(
    transaction: &mut Tx<'_>,
    export_id: Uuid,
    conversation_id: Uuid,
    ordinal: usize,
    message: &ParsedMessage,
) -> Result<Uuid, sqlx::Error> {
    let message_id: Uuid = sqlx::query_scalar(
        "INSERT INTO chatgpt_archive.messages
         (id, conversation_id, external_id, role, model_slug, source_ordinal,
          created_at, updated_at, provider_metadata)
         VALUES ($1, $2, $3, $4, $5, $6,
                 to_timestamp($7::double precision), to_timestamp($8::double precision), $9)
         ON CONFLICT (conversation_id, external_id) DO UPDATE
         SET role = EXCLUDED.role, model_slug = EXCLUDED.model_slug,
             source_ordinal = EXCLUDED.source_ordinal, created_at = EXCLUDED.created_at,
             updated_at = EXCLUDED.updated_at, provider_metadata = EXCLUDED.provider_metadata
         RETURNING id",
    )
    .bind(Uuid::now_v7())
    .bind(conversation_id)
    .bind(&message.external_id)
    .bind(message_role(&message.role))
    .bind(&message.model_slug)
    .bind(i32::try_from(ordinal).unwrap_or(i32::MAX))
    .bind(storable_epoch(message.created_at_epoch_seconds))
    .bind(storable_epoch(message.updated_at_epoch_seconds))
    .bind(&message.provider_metadata)
    .fetch_one(&mut **transaction)
    .await?;
    sqlx::query(
        "INSERT INTO chatgpt_archive.export_entity_observations
         (export_id, entity_kind, entity_id) VALUES ($1, 'message', $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(export_id)
    .bind(message_id)
    .execute(&mut **transaction)
    .await?;
    Ok(message_id)
}

/// Adds a content-part revision only when the parts differ from the latest one.
async fn persist_parts(
    transaction: &mut Tx<'_>,
    message_id: Uuid,
    parts: &[ParsedContentPart],
) -> Result<(), sqlx::Error> {
    let latest: Option<i32> = sqlx::query_scalar(
        "SELECT max(revision) FROM chatgpt_archive.content_parts WHERE message_id = $1",
    )
    .bind(message_id)
    .fetch_one(&mut **transaction)
    .await?;
    if let Some(revision) = latest {
        let stored: Vec<(i32, Value)> = sqlx::query_as(
            "SELECT ordinal, payload FROM chatgpt_archive.content_parts
             WHERE message_id = $1 AND revision = $2 ORDER BY ordinal",
        )
        .bind(message_id)
        .bind(revision)
        .fetch_all(&mut **transaction)
        .await?;
        let unchanged = stored.len() == parts.len()
            && stored.iter().zip(parts).all(|((ordinal, payload), part)| {
                usize::try_from(*ordinal).is_ok_and(|ordinal| ordinal == part.ordinal)
                    && *payload == part.payload
            });
        if unchanged {
            return Ok(());
        }
    }
    let revision = latest.map_or(0, |latest| latest.saturating_add(1));
    for part in parts {
        sqlx::query(
            "INSERT INTO chatgpt_archive.content_parts
             (id, message_id, revision, ordinal, part_kind, payload)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(Uuid::now_v7())
        .bind(message_id)
        .bind(revision)
        .bind(i32::try_from(part.ordinal).unwrap_or(i32::MAX))
        .bind(content_kind(&part.kind))
        .bind(&part.payload)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

/// The project row a conversation belongs to, as evidenced by the project list.
pub(crate) fn project_of_conversation(
    parsed: &ParsedConversations,
    projects: &BTreeMap<String, Uuid>,
    conversation_external_id: &str,
) -> Option<Uuid> {
    parsed
        .projects
        .iter()
        .find(|project| {
            project
                .conversation_external_ids
                .iter()
                .any(|id| id == conversation_external_id)
        })
        .and_then(|project| projects.get(&project.external_id).copied())
}

const fn message_role(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
        MessageRole::Internal => "internal",
        MessageRole::Unknown => "unknown",
    }
}

const fn content_kind(kind: &ContentPartKind) -> &'static str {
    match kind {
        ContentPartKind::Text => "text",
        ContentPartKind::ToolCall => "tool_call",
        ContentPartKind::ToolResult => "tool_result",
        ContentPartKind::Image => "image",
        ContentPartKind::File => "file",
        ContentPartKind::Unknown => "unknown",
    }
}
