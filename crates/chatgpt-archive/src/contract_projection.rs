//! Projects the persisted import into the `ai_archive.*` facts Knowledge consumes.
//!
//! The projection reads rows back out of the owned schema inside the transaction
//! that persisted them (XR-021 CONTRACTS.md section S07), so a fact can never
//! describe anything the archive does not durably hold. It never reuses the
//! in-memory parse. Identities are the persisted Ratatoskr UUIDs, content digests
//! come only from [`AiConversation::compute_content_digest`], and the owner is the
//! Platform user resolved by [`OwnerResolver`].
//!
//! The parsers are bound to a synthetic export schema. This projection is therefore
//! verified only against synthetic fixtures and makes no claim about real exports.

mod messages;

use ratatoskr_ai_archive_contracts::{
    AiArchiveCompleteness, AiArchiveImport, AiArchiveProvenance, AiCompletenessReport,
    AiConversation, AiConversationAdded, AiConversationUpdated, AiGap, AiProject, AiProjectAdded,
    AiProjectUpdated, AiProvider, AiText, AiTitle, ParserName, ParserVersion,
};
use ratatoskr_identifiers::{
    AiArchiveId, AiConversationId, AiProjectId, BlobRef, ContentDigest, DigestAlgorithm, DigestHex,
    EntityLocalId, Extensions, TenantRef, canonical_json,
};
use sha2::Digest as _;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::ParserId;
use crate::outbox::{NormalizedArchiveEvent, OutboxPlacement};
use crate::owner::OwnerResolver;
use crate::pg_instant::wire_instant;

type Tx<'a> = Transaction<'a, Postgres>;

/// Why a fact projection could not complete without exposing archive content.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProjectionError {
    /// Owned persistence failed.
    #[error("fact projection persistence failed")]
    Store(#[from] sqlx::Error),
    /// A durable value could not be encoded or decoded.
    #[error("fact projection evidence encoding failed")]
    Encode(#[from] serde_json::Error),
    /// A fact could not be constructed or stored.
    #[error("fact projection event failed")]
    Event(#[from] crate::OutboxError),
    /// A persisted value contradicts the contract it must satisfy.
    #[error("a persisted value cannot be represented by the archive contract")]
    Unrepresentable,
}

/// Whether the import head is emitted when no entity changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeadPolicy {
    /// The first import of an export always announces its head.
    Always,
    /// A reparse announces its head only together with a changed entity.
    WithChangedEntities,
}

/// The import a projection describes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FactScope<'a> {
    pub(crate) archive_id: Uuid,
    pub(crate) export_id: Uuid,
    pub(crate) account_id: Uuid,
    pub(crate) parser: &'a ParserId,
    pub(crate) head: HeadPolicy,
}

/// One fact built from rows, with the digest that decides whether it is news.
struct Candidate {
    event: NormalizedArchiveEvent,
    digest: ContentDigest,
}

/// Enqueues the facts of one persisted import, in contract order, and returns how
/// many rows were added.
///
/// Order: `archive.imported`, then each project, then each conversation. An entity
/// is `added` when no earlier fact exists for its identity, `updated` when its
/// content digest changed, and absent when it did not. An account without a
/// Platform mapping emits nothing at all.
///
/// # Errors
///
/// Returns [`ProjectionError`] when rows cannot be read or a fact cannot be built.
pub(crate) async fn project_import_facts(
    transaction: &mut Tx<'_>,
    owners: &OwnerResolver,
    scope: &FactScope<'_>,
) -> Result<usize, ProjectionError> {
    let Some(owner) = owners
        .owner_of_account(transaction, scope.account_id)
        .await?
    else {
        tracing::info!(
            reason = "account_without_platform_mapping",
            "ai_archive facts were not emitted"
        );
        return Ok(0);
    };
    let (import, head_digest) = load_import(transaction, scope, owner).await?;
    let provenance = AiArchiveProvenance::from_import(&import);
    let mut changed = project_candidates(transaction, scope, &provenance).await?;
    changed.extend(conversation_candidates(transaction, scope, &provenance).await?);

    let mut queue = Vec::new();
    if scope.head == HeadPolicy::Always || !changed.is_empty() {
        queue.push(Candidate {
            event: NormalizedArchiveEvent::archive_imported(&import)?,
            digest: head_digest,
        });
    }
    queue.extend(changed);
    let mut added = 0;
    for candidate in queue {
        let placement = OutboxPlacement {
            account_id: Some(scope.account_id),
            export_id: Some(scope.export_id),
            deduplication_key: Some(format!(
                "{}:{}:{}",
                candidate.event.event_type,
                candidate.event.aggregate_id,
                candidate.digest.hex.as_str()
            )),
        };
        if crate::Database::enqueue_normalized_event(transaction, &candidate.event, &placement)
            .await?
        {
            added += 1;
        }
    }
    Ok(added)
}

async fn load_import(
    transaction: &mut Tx<'_>,
    scope: &FactScope<'_>,
    owner: TenantRef,
) -> Result<(AiArchiveImport, ContentDigest), ProjectionError> {
    let blob_ref: serde_json::Value =
        sqlx::query_scalar("SELECT blob_ref FROM chatgpt_archive.exports WHERE id = $1")
            .bind(scope.export_id)
            .fetch_one(&mut **transaction)
            .await?;
    let source_export: BlobRef = serde_json::from_value(blob_ref)?;
    let imported_at = crate::pg_instant::database_now(transaction).await?;
    let report = completeness_report(transaction, scope).await?;
    let import = AiArchiveImport {
        ai_archive_id: AiArchiveId(scope.archive_id),
        provider: AiProvider::parse("chatgpt").map_err(|_| ProjectionError::Unrepresentable)?,
        owner,
        source_export,
        imported_at,
        parser_name: contract_parser_name(&scope.parser.name)?,
        parser_version: ParserVersion::parse(&scope.parser.version)
            .map_err(|_| ProjectionError::Unrepresentable)?,
        completeness_report: report,
        warnings: Vec::new(),
        extensions: Extensions::new(),
    };
    import
        .validate()
        .map_err(|_| ProjectionError::Unrepresentable)?;
    let digest = head_digest(&import)?;
    Ok((import, digest))
}

/// The head identity ignores `imported_at`, so repeating a pass adds no row.
fn head_digest(import: &AiArchiveImport) -> Result<ContentDigest, ProjectionError> {
    let mut stable = serde_json::to_value(import)?;
    if let Some(object) = stable.as_object_mut() {
        object.remove("imported_at");
    }
    digest_of(&stable)
}

/// The completeness the export's latest import run reported, with counts that are
/// re-derived from the rows the facts are built from.
async fn completeness_report(
    transaction: &mut Tx<'_>,
    scope: &FactScope<'_>,
) -> Result<AiCompletenessReport, ProjectionError> {
    let (status, counts, gaps): (String, serde_json::Value, serde_json::Value) = sqlx::query_as(
        "SELECT c.status, c.counts, c.gaps
             FROM chatgpt_archive.completeness_reports c
             JOIN chatgpt_archive.import_runs r ON r.id = c.import_run_id
             WHERE r.export_id = $1 ORDER BY r.started_at DESC, r.id DESC LIMIT 1",
    )
    .bind(scope.export_id)
    .fetch_one(&mut **transaction)
    .await?;
    let completeness: AiArchiveCompleteness =
        serde_json::from_value(serde_json::Value::String(status))?;
    let gaps: Vec<AiGap> = serde_json::from_value(gaps)?;
    let (conversations, messages): (i64, i64) = sqlx::query_as(
        "SELECT count(DISTINCT c.id), count(m.id)
         FROM chatgpt_archive.export_entity_observations o
         JOIN chatgpt_archive.conversations c ON c.id = o.entity_id
         LEFT JOIN chatgpt_archive.messages m ON m.conversation_id = c.id
         WHERE o.export_id = $1 AND o.entity_kind = 'conversation'",
    )
    .bind(scope.export_id)
    .fetch_one(&mut **transaction)
    .await?;
    let stored = |name: &str| {
        counts
            .get(name)
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or_default()
    };
    Ok(AiCompletenessReport {
        completeness,
        conversation_count: u32::try_from(conversations).unwrap_or(u32::MAX),
        message_count: u32::try_from(messages).unwrap_or(u32::MAX),
        asset_count: stored("assets"),
        gap_count: u32::try_from(gaps.len()).unwrap_or(u32::MAX),
        gaps,
    })
}

async fn project_candidates(
    transaction: &mut Tx<'_>,
    scope: &FactScope<'_>,
    base: &AiArchiveProvenance,
) -> Result<Vec<Candidate>, ProjectionError> {
    type Row = (
        Uuid,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT p.id, p.external_id, p.title, p.description, p.instructions,
                    p.parser_name, p.parser_version
             FROM chatgpt_archive.projects p
             JOIN chatgpt_archive.export_entity_observations o
               ON o.entity_kind = 'project' AND o.entity_id = p.id
             WHERE o.export_id = $1 ORDER BY p.external_id, p.id",
    )
    .bind(scope.export_id)
    .fetch_all(&mut **transaction)
    .await?;
    let mut candidates = Vec::new();
    for (id, external_id, title, description, instructions, parser_name, parser_version) in rows {
        let provenance =
            entity_provenance(base, parser_name.as_deref(), parser_version.as_deref())?;
        let project = AiProject {
            ai_project_id: AiProjectId(id),
            provider: provenance.provider.clone(),
            external_project_id: EntityLocalId::parse(&external_id).ok(),
            title: project_title(title.as_deref(), &external_id)?,
            description: description.and_then(|text| AiText::parse(&text).ok()),
            instructions: instructions.and_then(|text| AiText::parse(&text).ok()),
            provider_created_at: None,
            provider_updated_at: None,
            parser_name: provenance.parser_name.clone(),
            parser_version: provenance.parser_version.clone(),
            extensions: Extensions::new(),
        };
        let content_digest = digest_of(&serde_json::to_value(&project)?)?;
        let previous = last_emitted_digest(
            transaction,
            id,
            &[
                "ai_archive.project.added.v1",
                "ai_archive.project.updated.v1",
            ],
            &["payload", "content_digest", "hex"],
        )
        .await?;
        let import_provenance = provenance.clone();
        let event = match previous {
            Some(hex) if hex == content_digest.hex.as_str() => continue,
            Some(_) => NormalizedArchiveEvent::project_updated(&AiProjectUpdated {
                import_provenance,
                project,
                content_digest: content_digest.clone(),
                extensions: Extensions::new(),
            })?,
            None => NormalizedArchiveEvent::project_added(&AiProjectAdded {
                import_provenance,
                project,
                content_digest: content_digest.clone(),
                extensions: Extensions::new(),
            })?,
        };
        candidates.push(Candidate {
            event,
            digest: content_digest,
        });
    }
    Ok(candidates)
}

/// The import evidence stamped with the parser that last wrote an entity's rows.
///
/// A reparse that reads unchanged content leaves that stamp, and so the entity's
/// content digest, exactly as it was; only rewritten content takes the new parser.
/// Rows written before stamps existed fall back to the import's own parser.
fn entity_provenance(
    base: &AiArchiveProvenance,
    parser_name: Option<&str>,
    parser_version: Option<&str>,
) -> Result<AiArchiveProvenance, ProjectionError> {
    let mut provenance = base.clone();
    if let (Some(name), Some(version)) = (parser_name, parser_version) {
        provenance.parser_name = contract_parser_name(name)?;
        provenance.parser_version =
            ParserVersion::parse(version).map_err(|_| ProjectionError::Unrepresentable)?;
    }
    Ok(provenance)
}

/// A project is titled by its provider title, or by its provider id when the export
/// gave none, so the required contract title is never invented prose.
fn project_title(title: Option<&str>, external_id: &str) -> Result<AiTitle, ProjectionError> {
    title
        .and_then(|title| AiTitle::parse(title).ok())
        .or_else(|| AiTitle::parse(external_id).ok())
        .ok_or(ProjectionError::Unrepresentable)
}

async fn conversation_candidates(
    transaction: &mut Tx<'_>,
    scope: &FactScope<'_>,
    base: &AiArchiveProvenance,
) -> Result<Vec<Candidate>, ProjectionError> {
    let rows = messages::conversation_rows(transaction, scope.export_id).await?;
    let mut candidates = Vec::new();
    for row in rows {
        let provenance = entity_provenance(
            base,
            row.parser_name.as_deref(),
            row.parser_version.as_deref(),
        )?;
        let (messages, warnings) = messages::conversation_messages(
            transaction,
            row.id,
            &provenance.parser_name,
            &provenance.parser_version,
        )
        .await?;
        let content_digest = AiConversation::compute_content_digest(&messages)
            .map_err(|_| ProjectionError::Unrepresentable)?;
        let conversation = AiConversation {
            ai_conversation_id: AiConversationId(row.id),
            provider: provenance.provider.clone(),
            external_conversation_id: EntityLocalId::parse(&row.external_id).ok(),
            owner: provenance.owner,
            project_ref: row.project_id.map(|id| AiProjectId(id).as_entity_ref()),
            title: row.title.and_then(|title| AiTitle::parse(&title).ok()),
            provider_created_at: row.created_at.as_deref().and_then(wire_instant),
            provider_updated_at: row.updated_at.as_deref().and_then(wire_instant),
            messages,
            content_digest: content_digest.clone(),
            parser_name: provenance.parser_name.clone(),
            parser_version: provenance.parser_version.clone(),
            warnings,
            extensions: Extensions::new(),
        };
        let previous = last_emitted_digest(
            transaction,
            row.id,
            &[
                "ai_archive.conversation.added.v1",
                "ai_archive.conversation.updated.v1",
            ],
            &["payload", "conversation", "content_digest", "hex"],
        )
        .await?;
        let event = match previous {
            Some(hex) if hex == content_digest.hex.as_str() => continue,
            Some(_) => NormalizedArchiveEvent::conversation_updated(&AiConversationUpdated {
                import_provenance: provenance,
                conversation,
                extensions: Extensions::new(),
            })?,
            None => NormalizedArchiveEvent::conversation_added(&AiConversationAdded {
                import_provenance: provenance,
                conversation,
                extensions: Extensions::new(),
            })?,
        };
        candidates.push(Candidate {
            event,
            digest: content_digest,
        });
    }
    Ok(candidates)
}

/// The digest carried by the newest fact already queued for an aggregate, read at
/// `path` inside the stored envelope.
async fn last_emitted_digest(
    transaction: &mut Tx<'_>,
    aggregate_id: Uuid,
    event_types: &[&str],
    path: &[&str],
) -> Result<Option<String>, sqlx::Error> {
    let types: Vec<String> = event_types
        .iter()
        .map(|value| (*value).to_owned())
        .collect();
    let path: Vec<String> = path.iter().map(|value| (*value).to_owned()).collect();
    let digest: Option<Option<String>> = sqlx::query_scalar(
        "SELECT payload #>> $3::text[] FROM chatgpt_archive.outbox_events
         WHERE aggregate_id = $1 AND event_type = ANY($2) ORDER BY id DESC LIMIT 1",
    )
    .bind(aggregate_id)
    .bind(types)
    .bind(path)
    .fetch_optional(&mut **transaction)
    .await?;
    Ok(digest.flatten())
}

fn digest_of(value: &serde_json::Value) -> Result<ContentDigest, ProjectionError> {
    let canonical = canonical_json(value)?;
    let hex = DigestHex::parse(&hex::encode(sha2::Sha256::digest(canonical.as_bytes())))
        .map_err(|_| ProjectionError::Unrepresentable)?;
    Ok(ContentDigest {
        algorithm: DigestAlgorithm::Sha256,
        hex,
    })
}

/// The contract parser name for a registry name: lowercase snake case, at most 32
/// characters, starting with a letter. The registry spells names with hyphens.
fn contract_parser_name(name: &str) -> Result<ParserName, ProjectionError> {
    let mut spelled: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    spelled.truncate(32);
    ParserName::parse(&spelled).map_err(|_| ProjectionError::Unrepresentable)
}
