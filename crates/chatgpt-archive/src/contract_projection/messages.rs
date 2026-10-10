//! Reads a persisted conversation back as contract messages.
//!
//! The contract's author role is closed to four values. A message whose provider
//! role is internal or unknown cannot be attributed truthfully, so it is left out of
//! the fact, one conversation warning records that it was, and its children are
//! re-parented to the nearest represented ancestor. Part kinds that the contract has
//! no lossless typed form for travel as provider parts, verbatim.

use std::collections::{BTreeMap, BTreeSet};

use ratatoskr_ai_archive_contracts::{
    AiAuthorRole, AiContentPart, AiMessage, AiModelName, AiText, ParserName, ParserVersion,
};
use ratatoskr_error_contracts::{ErrorCode, WarningEnvelope};
use ratatoskr_identifiers::{EntityLocalId, Extensions, SafeMessage};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::ProjectionError;
use crate::pg_instant::{pg_instant_sql, wire_instant};

type Tx<'a> = Transaction<'a, Postgres>;

const ROLE_WARNING_CODE: &str = "ai_archive.message_role_unrepresentable";
const ROLE_WARNING_MESSAGE: &str =
    "Messages with an internal or unknown provider role were left out of this conversation.";
const IDENTITY_WARNING_CODE: &str = "ai_archive.message_id_unrepresentable";
const IDENTITY_WARNING_MESSAGE: &str =
    "Messages without a representable provider identity were left out of this conversation.";

/// One persisted conversation header.
pub(super) struct ConversationRow {
    pub(super) id: Uuid,
    pub(super) external_id: String,
    pub(super) title: Option<String>,
    pub(super) project_id: Option<Uuid>,
    pub(super) created_at: Option<String>,
    pub(super) updated_at: Option<String>,
    /// The parser that last wrote this conversation's rows.
    pub(super) parser_name: Option<String>,
    pub(super) parser_version: Option<String>,
}

/// One conversation header as the query returns it.
type HeaderRow = (
    Uuid,
    String,
    Option<String>,
    Option<Uuid>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// The conversations an export observed, in provider id order.
pub(super) async fn conversation_rows(
    transaction: &mut Tx<'_>,
    export_id: Uuid,
) -> Result<Vec<ConversationRow>, sqlx::Error> {
    let query = format!(
        "SELECT c.id, c.external_id, c.title, c.project_id, {created}, {updated},
                c.parser_name, c.parser_version
         FROM chatgpt_archive.conversations c
         JOIN chatgpt_archive.export_entity_observations o
           ON o.entity_kind = 'conversation' AND o.entity_id = c.id
         WHERE o.export_id = $1 ORDER BY c.external_id, c.id",
        created = pg_instant_sql("c.provider_created_at"),
        updated = pg_instant_sql("c.provider_updated_at"),
    );
    let rows: Vec<HeaderRow> = sqlx::query_as(&query)
        .bind(export_id)
        .fetch_all(&mut **transaction)
        .await?;
    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                external_id,
                title,
                project_id,
                created_at,
                updated_at,
                parser_name,
                parser_version,
            )| ConversationRow {
                id,
                external_id,
                title,
                project_id,
                created_at,
                updated_at,
                parser_name,
                parser_version,
            },
        )
        .collect())
}

/// One message as the query returns it.
type MessageHeader = (
    Uuid,
    Option<String>,
    Option<Uuid>,
    String,
    Option<String>,
    Option<String>,
);

/// One persisted message before role and identity filtering.
struct MessageRow {
    id: Uuid,
    external_id: Option<String>,
    parent: Option<Uuid>,
    role: String,
    model: Option<String>,
    created_at: Option<String>,
}

/// The represented messages of a conversation in provider order, plus the warnings
/// that describe what was left out.
pub(super) async fn conversation_messages(
    transaction: &mut Tx<'_>,
    conversation_id: Uuid,
    parser_name: &ParserName,
    parser_version: &ParserVersion,
) -> Result<(Vec<AiMessage>, Vec<WarningEnvelope>), ProjectionError> {
    let rows = message_rows(transaction, conversation_id).await?;
    let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
    let parts = message_parts(transaction, &ids).await?;
    let by_id: BTreeMap<Uuid, &MessageRow> = rows.iter().map(|row| (row.id, row)).collect();

    let mut messages = Vec::new();
    let mut role_left_out = false;
    let mut identity_left_out = false;
    for row in &rows {
        let Some(role) = author_role(&row.role) else {
            role_left_out = true;
            continue;
        };
        let Some(external_id) = row
            .external_id
            .as_deref()
            .and_then(|id| EntityLocalId::parse(id).ok())
        else {
            identity_left_out = true;
            continue;
        };
        messages.push(AiMessage {
            external_message_id: external_id,
            author_role: role,
            parent_message_id: nearest_represented_parent(row, &by_id),
            parts: parts.get(&row.id).cloned().unwrap_or_default(),
            model: row
                .model
                .as_deref()
                .and_then(|model| AiModelName::parse(model).ok()),
            provider_created_at: row.created_at.as_deref().and_then(wire_instant),
            parser_name: parser_name.clone(),
            parser_version: parser_version.clone(),
            extensions: Extensions::new(),
        });
    }
    let mut warnings = Vec::new();
    if role_left_out {
        warnings.push(warning(ROLE_WARNING_CODE, ROLE_WARNING_MESSAGE)?);
    }
    if identity_left_out {
        warnings.push(warning(IDENTITY_WARNING_CODE, IDENTITY_WARNING_MESSAGE)?);
    }
    Ok((messages, warnings))
}

fn warning(code: &str, message: &str) -> Result<WarningEnvelope, ProjectionError> {
    Ok(WarningEnvelope {
        code: ErrorCode::parse(code).map_err(|_| ProjectionError::Unrepresentable)?,
        message: SafeMessage::parse(message).map_err(|_| ProjectionError::Unrepresentable)?,
        field_path: None,
        extensions: Extensions::new(),
    })
}

fn author_role(role: &str) -> Option<AiAuthorRole> {
    match role {
        "user" => Some(AiAuthorRole::User),
        "assistant" => Some(AiAuthorRole::Assistant),
        "system" => Some(AiAuthorRole::System),
        "tool" => Some(AiAuthorRole::Tool),
        _ => None,
    }
}

/// The provider id of the nearest ancestor that is itself represented.
fn nearest_represented_parent(
    row: &MessageRow,
    by_id: &BTreeMap<Uuid, &MessageRow>,
) -> Option<EntityLocalId> {
    let mut seen = BTreeSet::from([row.id]);
    let mut cursor = row.parent;
    while let Some(parent_id) = cursor {
        if !seen.insert(parent_id) {
            return None;
        }
        let parent = by_id.get(&parent_id)?;
        let represented = author_role(&parent.role).is_some();
        if represented && let Some(external_id) = parent.external_id.as_deref() {
            return EntityLocalId::parse(external_id).ok();
        }
        cursor = parent.parent;
    }
    None
}

async fn message_rows(
    transaction: &mut Tx<'_>,
    conversation_id: Uuid,
) -> Result<Vec<MessageRow>, sqlx::Error> {
    let query = format!(
        "SELECT m.id, m.external_id, m.parent_message_id, m.role, m.model_slug, {created}
         FROM chatgpt_archive.messages m WHERE m.conversation_id = $1
         ORDER BY m.source_ordinal NULLS LAST, m.created_at NULLS LAST, m.external_id, m.id",
        created = pg_instant_sql("m.created_at"),
    );
    let rows: Vec<MessageHeader> = sqlx::query_as(&query)
        .bind(conversation_id)
        .fetch_all(&mut **transaction)
        .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, external_id, parent, role, model, created_at)| MessageRow {
                id,
                external_id,
                parent,
                role,
                model,
                created_at,
            },
        )
        .collect())
}

/// The parts of each message at its latest revision, in provider order.
async fn message_parts(
    transaction: &mut Tx<'_>,
    message_ids: &[Uuid],
) -> Result<BTreeMap<Uuid, Vec<AiContentPart>>, sqlx::Error> {
    let rows: Vec<(Uuid, String, Value)> = sqlx::query_as(
        "SELECT p.message_id, p.part_kind, p.payload
         FROM chatgpt_archive.content_parts p
         WHERE p.message_id = ANY($1)
           AND p.revision = (SELECT max(x.revision) FROM chatgpt_archive.content_parts x
                             WHERE x.message_id = p.message_id)
         ORDER BY p.message_id, p.ordinal",
    )
    .bind(message_ids)
    .fetch_all(&mut **transaction)
    .await?;
    let mut parts: BTreeMap<Uuid, Vec<AiContentPart>> = BTreeMap::new();
    for (message_id, kind, payload) in rows {
        parts
            .entry(message_id)
            .or_default()
            .push(content_part(&kind, &payload));
    }
    Ok(parts)
}

/// Plain text becomes a text part. Every other shape has no lossless typed form in
/// the contract, so it is carried verbatim as a provider part that consumers
/// preserve and render generically.
fn content_part(kind: &str, payload: &Value) -> AiContentPart {
    if let (true, Value::String(text)) = (kind == "text", payload)
        && let Ok(text) = AiText::parse(text)
    {
        return AiContentPart::Text { text };
    }
    AiContentPart::Unknown(json!({
        "part_kind": "provider_part",
        "provider_part_kind": kind,
        "payload": payload,
    }))
}
