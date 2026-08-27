//! Portable archive export boundary.

use std::collections::BTreeMap;
use std::io::{Cursor, Write as _};
use std::path::Path;

use serde_json::{Map, Value, json};
use sha2::Digest as _;
use zip::write::SimpleFileOptions;

use crate::BlobStore;
use crate::Database;

/// Deterministic portable archive writer.
#[derive(Debug, Default)]
pub struct PortableArchiveExporter;

impl PortableArchiveExporter {
    /// Creates an exporter.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Produces a portable archive for one selected state.
    ///
    /// # Errors
    ///
    /// Returns [`PortableExportError`] when output cannot be produced.
    pub fn export_to_bytes(
        &self,
        state: &PortableArchiveState,
    ) -> Result<Vec<u8>, PortableExportError> {
        archive_bytes(state, normalized_members(state)?, None)
    }

    /// Produces a portable archive, including selected asset bytes.
    ///
    /// # Errors
    ///
    /// Returns [`PortableExportError`] when output cannot be produced.
    pub async fn export_to_bytes_with_assets(
        &self,
        state: &PortableArchiveState,
        blob_store: &BlobStore,
    ) -> Result<Vec<u8>, PortableExportError> {
        let mut members = normalized_members(state)?;
        for asset in &state.assets {
            if asset.availability != PortableAssetAvailability::Verified {
                continue;
            }
            let reference = asset
                .blob
                .as_ref()
                .ok_or(PortableExportError::MissingVerifiedAssetReference)?;
            let source = blob_store.verify(reference).await?;
            let bytes = tokio::fs::read(source).await?;
            members.push(Member {
                path: format!("assets/{}", path_component(&asset.external_id)),
                bytes,
            });
        }
        archive_bytes(state, members, None)
    }

    /// Writes a verified portable archive atomically to `output`.
    ///
    /// # Errors
    ///
    /// Returns [`PortableExportError`] when asset verification or output
    /// publication fails; no completed output is left on failure.
    pub async fn export_to_path_with_assets(
        &self,
        state: &PortableArchiveState,
        blob_store: &BlobStore,
        output: &Path,
    ) -> Result<(), PortableExportError> {
        let bytes = self.export_to_bytes_with_assets(state, blob_store).await?;
        let parent = output
            .parent()
            .ok_or(PortableExportError::InvalidOutputPath)?;
        tokio::fs::create_dir_all(parent).await?;
        let temporary = parent.join(format!(".portable-export-{}.part", uuid::Uuid::now_v7()));
        tokio::fs::write(&temporary, bytes).await?;
        tokio::fs::rename(&temporary, output).await?;
        Ok(())
    }

    /// Produces a portable archive from the requested state collection.
    ///
    /// # Errors
    ///
    /// Returns [`PortableExportError`] when no state can be selected or output
    /// cannot be produced.
    pub fn export_selected_to_bytes(
        &self,
        states: &[PortableArchiveState],
        filter: &PortableExportFilter,
    ) -> Result<Vec<u8>, PortableExportError> {
        let state = states
            .iter()
            .find(|state| state.account_external_ref == filter.account_external_ref)
            .ok_or(PortableExportError::EmptySelection)?;
        let selected = select_state(state, filter);
        archive_bytes(&selected, normalized_members(&selected)?, Some(filter))
    }
}

/// Export failure.
#[derive(Debug, thiserror::Error)]
pub enum PortableExportError {
    /// The exporter could not encode an archive member.
    #[error("portable archive JSON encoding failed")]
    Json(#[from] serde_json::Error),
    /// The ZIP stream could not be written.
    #[error("portable archive ZIP encoding failed")]
    Zip(#[from] zip::result::ZipError),
    /// The in-memory ZIP stream could not be written.
    #[error("portable archive ZIP I/O failed")]
    Io(#[from] std::io::Error),
    /// A verified asset did not carry a usable blob reference.
    #[error("a verified asset did not carry a usable blob reference")]
    MissingVerifiedAssetReference,
    /// A `BlobStore` read failed closed.
    #[error("a verified asset could not be read")]
    Blob(#[from] crate::BlobStoreError),
    /// The requested filters selected no tenant state.
    #[error("the requested export filters selected no archive state")]
    EmptySelection,
    /// The destination has no parent directory component.
    #[error("the portable archive output path has no parent directory")]
    InvalidOutputPath,
    /// The persistence read model could not load selected evidence.
    #[error("the portable archive read model failed")]
    Store(#[from] sqlx::Error),
}

/// Tenant-scoped normalized evidence for one export.
#[derive(Debug, Clone, PartialEq)]
pub struct PortableArchiveState {
    /// Owning account external reference.
    pub account_external_ref: String,
    /// Immutable source provenance.
    pub provenance: PortableProvenance,
    /// Selected projects.
    pub projects: Vec<PortableProject>,
    /// Selected conversations.
    pub conversations: Vec<PortableConversation>,
    /// Selected asset references.
    pub assets: Vec<PortableAsset>,
}

/// Tenant and evidence predicates for a portable export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableExportFilter {
    /// Required owning account reference.
    pub account_external_ref: String,
    /// Optional exact provider project identifier.
    pub project_external_id: Option<String>,
    /// Optional inclusive lower observed-time bound as RFC 3339 text.
    pub observed_from_rfc3339: Option<String>,
    /// Optional inclusive upper observed-time bound as RFC 3339 text.
    pub observed_to_rfc3339: Option<String>,
}

impl Database {
    /// Loads one tenant's persisted normalized archive state for portable export.
    ///
    /// # Errors
    ///
    /// Returns [`PortableExportError`] when the account has no selected source
    /// evidence or `PostgreSQL` cannot load the owned projections.
    pub async fn load_portable_archive_state(
        &self,
        filter: &PortableExportFilter,
    ) -> Result<PortableArchiveState, PortableExportError> {
        let provenance = sqlx::query_as::<_, (String, String, String, String)>(
            "SELECT a.external_ref, e.sha256_hex, COALESCE(r.parser_version, 'unknown'), to_char(e.received_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') FROM chatgpt_archive.accounts a JOIN chatgpt_archive.exports e ON e.account_id = a.id LEFT JOIN LATERAL (SELECT parser_version FROM chatgpt_archive.import_runs WHERE export_id = e.id ORDER BY started_at DESC LIMIT 1) r ON TRUE WHERE a.external_ref = $1 ORDER BY e.received_at DESC LIMIT 1",
        )
        .bind(&filter.account_external_ref)
        .fetch_optional(self.pool())
        .await?
        .ok_or(PortableExportError::EmptySelection)?;
        let projects = load_projects(self, filter).await?;
        let conversations = load_conversations(self, filter).await?;
        let assets = load_assets(self, filter).await?;
        Ok(PortableArchiveState {
            account_external_ref: provenance.0,
            provenance: PortableProvenance {
                archive_sha256: provenance.1,
                parser_name: "persisted-normalized-state".to_owned(),
                parser_version: provenance.2,
                observed_at_rfc3339: provenance.3,
            },
            projects,
            conversations,
            assets,
        })
    }
}

async fn load_projects(
    database: &Database,
    filter: &PortableExportFilter,
) -> Result<Vec<PortableProject>, PortableExportError> {
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>, bool, String)>(
        "SELECT p.external_id, p.title, p.description, p.instructions, p.archived_observed, to_char(COALESCE(e.received_at, p.updated_at) AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') FROM chatgpt_archive.projects p JOIN chatgpt_archive.accounts a ON a.id = p.account_id LEFT JOIN chatgpt_archive.exports e ON e.id = p.last_seen_export WHERE a.external_ref = $1 AND ($2::text IS NULL OR p.external_id = $2) AND ($3::text IS NULL OR COALESCE(e.received_at, p.updated_at) >= $3::timestamptz) AND ($4::text IS NULL OR COALESCE(e.received_at, p.updated_at) <= $4::timestamptz) ORDER BY p.external_id",
    )
    .bind(&filter.account_external_ref)
    .bind(&filter.project_external_id)
    .bind(&filter.observed_from_rfc3339)
    .bind(&filter.observed_to_rfc3339)
    .fetch_all(database.pool())
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(
                external_id,
                title,
                description,
                instructions,
                archived_observed,
                observed_at_rfc3339,
            )| PortableProject {
                payload: json!({
                    "external_id": external_id,
                    "title": title,
                    "description": description,
                    "instructions": instructions,
                    "archived_observed": archived_observed,
                }),
                external_id,
                title,
                observed_at_rfc3339,
            },
        )
        .collect())
}

async fn load_conversations(
    database: &Database,
    filter: &PortableExportFilter,
) -> Result<Vec<PortableConversation>, PortableExportError> {
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>, String, String, String)>(
        "SELECT c.external_id, p.external_id, c.title, c.conversation_kind, to_char(COALESCE(e.received_at, c.updated_at) AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), COALESCE(jsonb_agg(jsonb_build_object('external_id', m.external_id, 'parent_external_id', parent.external_id, 'role', m.role, 'model_slug', m.model_slug, 'generation_index', m.generation_index, 'interrupted', m.interrupted, 'created_at_rfc3339', CASE WHEN m.created_at IS NULL THEN NULL ELSE to_char(m.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') END, 'updated_at_rfc3339', CASE WHEN m.updated_at IS NULL THEN NULL ELSE to_char(m.updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') END, 'provider_metadata', m.provider_metadata, 'parts', COALESCE(parts.parts, '[]'::jsonb)) ORDER BY m.created_at NULLS FIRST, m.id) FILTER (WHERE m.id IS NOT NULL), '[]'::jsonb)::text FROM chatgpt_archive.conversations c JOIN chatgpt_archive.accounts a ON a.id = c.account_id LEFT JOIN chatgpt_archive.projects p ON p.id = c.project_id LEFT JOIN chatgpt_archive.exports e ON e.id = c.last_seen_export LEFT JOIN chatgpt_archive.messages m ON m.conversation_id = c.id LEFT JOIN chatgpt_archive.messages parent ON parent.id = m.parent_message_id LEFT JOIN LATERAL (SELECT jsonb_agg(jsonb_build_object('revision', cp.revision, 'ordinal', cp.ordinal, 'part_kind', cp.part_kind, 'payload', cp.payload, 'blob_ref', cp.blob_ref) ORDER BY cp.revision, cp.ordinal)::jsonb AS parts FROM chatgpt_archive.content_parts cp WHERE cp.message_id = m.id) parts ON TRUE WHERE a.external_ref = $1 AND ($2::text IS NULL OR p.external_id = $2) AND ($3::text IS NULL OR COALESCE(e.received_at, c.updated_at) >= $3::timestamptz) AND ($4::text IS NULL OR COALESCE(e.received_at, c.updated_at) <= $4::timestamptz) GROUP BY c.id, p.id, e.received_at ORDER BY c.external_id",
    )
    .bind(&filter.account_external_ref)
    .bind(&filter.project_external_id)
    .bind(&filter.observed_from_rfc3339)
    .bind(&filter.observed_to_rfc3339)
    .fetch_all(database.pool())
    .await?;
    rows.into_iter()
        .map(
            |(
                external_id,
                project_external_id,
                title,
                conversation_kind,
                observed_at_rfc3339,
                messages,
            )| {
                Ok(PortableConversation {
                    payload: json!({
                        "external_id": external_id,
                        "project_external_id": project_external_id,
                        "title": title,
                        "conversation_kind": conversation_kind,
                        "messages": serde_json::from_str::<Value>(&messages)?,
                    }),
                    external_id,
                    project_external_id,
                    title,
                    observed_at_rfc3339,
                })
            },
        )
        .collect()
}

async fn load_assets(
    database: &Database,
    filter: &PortableExportFilter,
) -> Result<Vec<PortableAsset>, PortableExportError> {
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<serde_json::Value>, bool, String)>(
        "SELECT s.external_id, s.media_type, s.blob_ref, s.locally_backed_up, to_char(e.received_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') FROM chatgpt_archive.assets s JOIN chatgpt_archive.exports e ON e.id = s.observed_in JOIN chatgpt_archive.accounts a ON a.id = e.account_id WHERE a.external_ref = $1 AND ($2::text IS NULL) AND ($3::text IS NULL OR e.received_at >= $3::timestamptz) AND ($4::text IS NULL OR e.received_at <= $4::timestamptz) ORDER BY s.external_id",
    )
    .bind(&filter.account_external_ref)
    .bind(&filter.project_external_id)
    .bind(&filter.observed_from_rfc3339)
    .bind(&filter.observed_to_rfc3339)
    .fetch_all(database.pool())
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(external_id, media_type, blob, locally_backed_up, observed_at_rfc3339)| {
                let blob = blob.and_then(|value| serde_json::from_value(value).ok());
                let availability = if locally_backed_up && blob.is_some() {
                    PortableAssetAvailability::Verified
                } else {
                    PortableAssetAvailability::Missing
                };
                PortableAsset {
                    external_id,
                    project_external_id: None,
                    observed_at_rfc3339,
                    availability,
                    blob,
                    media_type,
                }
            },
        )
        .collect())
}

/// Non-sensitive source identity carried by every rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableProvenance {
    /// SHA-256 digest of the original provider archive.
    pub archive_sha256: String,
    /// Parser name.
    pub parser_name: String,
    /// Parser version.
    pub parser_version: String,
    /// RFC 3339 observation timestamp.
    pub observed_at_rfc3339: String,
}

/// One normalized project record.
#[derive(Debug, Clone, PartialEq)]
pub struct PortableProject {
    /// Provider project identity.
    pub external_id: String,
    /// Project title.
    pub title: Option<String>,
    /// RFC 3339 observation timestamp.
    pub observed_at_rfc3339: String,
    /// Complete normalized evidence.
    pub payload: Value,
}

/// One normalized conversation record.
#[derive(Debug, Clone, PartialEq)]
pub struct PortableConversation {
    /// Provider conversation identity.
    pub external_id: String,
    /// Linked provider project identity when observed.
    pub project_external_id: Option<String>,
    /// Conversation title.
    pub title: Option<String>,
    /// RFC 3339 observation timestamp.
    pub observed_at_rfc3339: String,
    /// Complete normalized evidence.
    pub payload: Value,
}

/// One selected asset reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableAsset {
    /// Provider asset identity.
    pub external_id: String,
    /// Linked provider project identity when observed.
    pub project_external_id: Option<String>,
    /// RFC 3339 observation timestamp.
    pub observed_at_rfc3339: String,
    /// Whether the source archive verified local bytes.
    pub availability: PortableAssetAvailability,
    /// Verified source bytes when available.
    pub blob: Option<ratatoskr_identifiers::BlobRef>,
    /// Media type observed for the asset.
    pub media_type: Option<String>,
}

/// Whether asset bytes are available for a portable export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortableAssetAvailability {
    /// Bytes were verified and can be copied.
    Verified,
    /// The provider named an asset without locally backed-up bytes.
    Missing,
    /// The candidate bytes failed a security or integrity check.
    Quarantined,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Member {
    path: String,
    bytes: Vec<u8>,
}

fn normalized_members(state: &PortableArchiveState) -> Result<Vec<Member>, PortableExportError> {
    let mut members = Vec::new();
    let mut projects = state.projects.iter().collect::<Vec<_>>();
    projects.sort_by(|left, right| left.external_id.cmp(&right.external_id));
    for project in projects {
        let value = json!({
            "provenance": provenance_value(&state.provenance),
            "project": {
                "external_id": project.external_id,
                "title": project.title,
                "payload": canonicalize(&project.payload),
            },
        });
        let component = path_component(&project.external_id);
        members.push(Member {
            path: format!("projects/{component}.json"),
            bytes: canonical_json(&value)?,
        });
        members.push(Member {
            path: format!("projects/{component}.md"),
            bytes: project_markdown(project, &state.provenance).into_bytes(),
        });
    }

    let mut conversations = state.conversations.iter().collect::<Vec<_>>();
    conversations.sort_by(|left, right| left.external_id.cmp(&right.external_id));
    for conversation in conversations {
        let value = json!({
            "provenance": provenance_value(&state.provenance),
            "conversation": {
                "external_id": conversation.external_id,
                "project_external_id": conversation.project_external_id,
                "title": conversation.title,
                "observed_at_rfc3339": conversation.observed_at_rfc3339,
                "payload": canonicalize(&conversation.payload),
            },
        });
        let component = path_component(&conversation.external_id);
        members.push(Member {
            path: format!("conversations/{component}.json"),
            bytes: canonical_json(&value)?,
        });
        members.push(Member {
            path: format!("conversations/{component}.md"),
            bytes: conversation_markdown(conversation, &state.provenance).into_bytes(),
        });
    }
    members.sort();
    Ok(members)
}

fn archive_bytes(
    state: &PortableArchiveState,
    mut members: Vec<Member>,
    filter: Option<&PortableExportFilter>,
) -> Result<Vec<u8>, PortableExportError> {
    members.sort();
    let manifest = manifest_member(state, &members, filter)?;
    members.push(manifest);
    write_zip(members)
}

fn manifest_member(
    state: &PortableArchiveState,
    members: &[Member],
    filter: Option<&PortableExportFilter>,
) -> Result<Member, PortableExportError> {
    let members = members
        .iter()
        .map(|member| {
            json!({
                "path": member.path,
                "sha256": hex::encode(sha2::Sha256::digest(&member.bytes)),
                "length_bytes": member.bytes.len(),
                "media_type": member_media_type(&member.path, state),
                "provenance": provenance_value(&state.provenance),
            })
        })
        .collect::<Vec<_>>();
    let warnings = state
        .assets
        .iter()
        .filter(|asset| asset.availability != PortableAssetAvailability::Verified)
        .map(|asset| {
            json!({
                "asset_external_id": asset.external_id,
                "availability": availability_name(asset.availability),
            })
        })
        .collect::<Vec<_>>();
    Ok(Member {
        path: "manifest.json".to_owned(),
        bytes: canonical_json(&json!({
            "format": "ratatoskr-portable-archive",
            "account_external_ref": state.account_external_ref,
            "provenance": provenance_value(&state.provenance),
            "filters": filter.map(filter_value),
            "members": members,
            "asset_warnings": warnings,
        }))?,
    })
}

fn filter_value(filter: &PortableExportFilter) -> Value {
    json!({
        "account_external_ref": filter.account_external_ref,
        "project_external_id": filter.project_external_id,
        "observed_from_rfc3339": filter.observed_from_rfc3339,
        "observed_to_rfc3339": filter.observed_to_rfc3339,
    })
}

fn select_state(
    state: &PortableArchiveState,
    filter: &PortableExportFilter,
) -> PortableArchiveState {
    let matches = |project_external_id: Option<&str>, observed_at: &str| {
        filter
            .project_external_id
            .as_deref()
            .is_none_or(|project| project_external_id == Some(project))
            && filter
                .observed_from_rfc3339
                .as_deref()
                .is_none_or(|from| observed_at >= from)
            && filter
                .observed_to_rfc3339
                .as_deref()
                .is_none_or(|to| observed_at <= to)
    };
    PortableArchiveState {
        account_external_ref: state.account_external_ref.clone(),
        provenance: state.provenance.clone(),
        projects: state
            .projects
            .iter()
            .filter(|project| matches(Some(&project.external_id), &project.observed_at_rfc3339))
            .cloned()
            .collect(),
        conversations: state
            .conversations
            .iter()
            .filter(|conversation| {
                matches(
                    conversation.project_external_id.as_deref(),
                    &conversation.observed_at_rfc3339,
                )
            })
            .cloned()
            .collect(),
        assets: state
            .assets
            .iter()
            .filter(|asset| {
                matches(
                    asset.project_external_id.as_deref(),
                    &asset.observed_at_rfc3339,
                )
            })
            .cloned()
            .collect(),
    }
}

fn member_media_type(path: &str, state: &PortableArchiveState) -> Option<String> {
    let asset = state
        .assets
        .iter()
        .find(|asset| format!("assets/{}", path_component(&asset.external_id)) == path)?;
    asset.media_type.clone()
}

fn canonical_json(value: &Value) -> Result<Vec<u8>, PortableExportError> {
    let mut bytes = serde_json::to_vec(&canonicalize(value))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonicalize).collect()),
        Value::Object(items) => Value::Object(Map::from_iter(
            items
                .iter()
                .map(|(key, value)| (key.clone(), canonicalize(value)))
                .collect::<BTreeMap<_, _>>(),
        )),
        scalar => scalar.clone(),
    }
}

fn provenance_value(provenance: &PortableProvenance) -> Value {
    json!({
        "archive_sha256": provenance.archive_sha256,
        "parser_name": provenance.parser_name,
        "parser_version": provenance.parser_version,
        "observed_at_rfc3339": provenance.observed_at_rfc3339,
    })
}

fn provenance_header(provenance: &PortableProvenance) -> String {
    format!(
        "<!-- provenance: archive-sha256={}; parser={}; parser-version={} -->\n\n",
        provenance.archive_sha256, provenance.parser_name, provenance.parser_version
    )
}

fn project_markdown(project: &PortableProject, provenance: &PortableProvenance) -> String {
    let title = project.title.as_deref().unwrap_or("Untitled project");
    let description = project
        .payload
        .get("description")
        .and_then(Value::as_str)
        .map(|description| format!("\n## Description\n\n{description}\n"))
        .unwrap_or_default();
    let instructions = project
        .payload
        .get("instructions")
        .and_then(Value::as_str)
        .map(|instructions| format!("\n## Instructions\n\n{instructions}\n"))
        .unwrap_or_default();
    format!(
        "{}# {}\n\n`project_id`: `{}`\n",
        provenance_header(provenance),
        title,
        project.external_id
    ) + &description
        + &instructions
}

fn conversation_markdown(
    conversation: &PortableConversation,
    provenance: &PortableProvenance,
) -> String {
    let title = conversation
        .title
        .as_deref()
        .unwrap_or("Untitled conversation");
    let mut markdown = format!(
        "{}# {}\n\n`conversation_id`: `{}`\n\n`observed_at`: `{}`\n",
        provenance_header(provenance),
        title,
        conversation.external_id,
        conversation.observed_at_rfc3339
    );
    append_conversation_messages(&mut markdown, &conversation.payload);
    markdown
}

fn append_conversation_messages(markdown: &mut String, payload: &Value) {
    let Some(messages) = payload.get("messages").and_then(Value::as_array) else {
        return;
    };
    for message in messages {
        append_message_markdown(markdown, message);
    }
}

fn append_message_markdown(markdown: &mut String, message: &Value) {
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    markdown.push_str("\n## ");
    markdown.push_str(role);
    markdown.push('\n');
    let Some(parts) = message.get("parts").and_then(Value::as_array) else {
        return;
    };
    for part in parts {
        append_part_markdown(markdown, part);
    }
}

fn append_part_markdown(markdown: &mut String, part: &Value) {
    let payload = part.get("payload").unwrap_or(&Value::Null);
    let text = payload
        .as_str()
        .or_else(|| payload.get("text").and_then(Value::as_str));
    if let Some(text) = text {
        markdown.push('\n');
        markdown.push_str(text);
        markdown.push('\n');
        return;
    }
    markdown.push_str("\n```json\n");
    markdown.push_str(&canonicalize(payload).to_string());
    markdown.push_str("\n```\n");
}

fn path_component(external_id: &str) -> String {
    let readable = external_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let digest = hex::encode(sha2::Sha256::digest(external_id.as_bytes()));
    let suffix = digest.chars().take(16).collect::<String>();
    format!("{readable}-{suffix}")
}

fn availability_name(availability: PortableAssetAvailability) -> &'static str {
    match availability {
        PortableAssetAvailability::Verified => "verified",
        PortableAssetAvailability::Missing => "missing",
        PortableAssetAvailability::Quarantined => "quarantined",
    }
}

fn write_zip(members: Vec<Member>) -> Result<Vec<u8>, PortableExportError> {
    let cursor = Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::DEFAULT)
        .unix_permissions(0o644);
    for member in members {
        writer.start_file(member.path, options)?;
        writer.write_all(&member.bytes)?;
    }
    Ok(writer.finish()?.into_inner())
}
