//! Facts for the records a deletion keeps (XR-021 CONTRACTS.md R3-06).
//!
//! A project or conversation is announced under the archive of the export that first saw it, and
//! a later export that sees the same record announces nothing (its digest is unchanged). When a
//! deletion tombstones that archive, Knowledge drops everything announced under it, including
//! what a retained export still holds. Each such survivor is announced again as an update whose
//! provenance names the earliest retained export that presented the content the record holds,
//! stamped after the tombstone so a consumer applies it afterwards. The content itself was put
//! back to a version a retained export presented before this runs (`privacy_deletion::restore`).

use ratatoskr_ai_archive_contracts::{
    AiArchiveProvenance, AiConversationUpdated, AiProjectUpdated, AiProvider, ParserName,
    ParserVersion,
};
use ratatoskr_identifiers::{BlobRef, Extensions, TenantRef, WireTimestamp};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::{
    PROJECT_SELECT, ProjectFact, ProjectRow, ProjectionError, contract_parser_name,
    conversation_fact, messages, project_fact,
};
use crate::outbox::{NormalizedArchiveEvent, OutboxPlacement};
use crate::pg_instant::{pg_instant_sql, wire_instant};

type Tx<'a> = Transaction<'a, Postgres>;

const PROJECT_TYPES: [&str; 2] = [
    "ai_archive.project.added.v1",
    "ai_archive.project.updated.v1",
];
const CONVERSATION_TYPES: [&str; 2] = [
    "ai_archive.conversation.added.v1",
    "ai_archive.conversation.updated.v1",
];
const ARCHIVE_IMPORTED: &str = "ai_archive.archive.imported.v1";

/// The retained records a deletion keeps whose latest announced fact names a deleted archive.
#[derive(Debug, Default)]
pub(crate) struct Survivors {
    projects: Vec<Uuid>,
    conversations: Vec<Uuid>,
}

impl Survivors {
    pub(crate) fn is_empty(&self) -> bool {
        self.projects.is_empty() && self.conversations.is_empty()
    }
}

/// The deletion whose survivors are announced again.
pub(crate) struct Deletion {
    /// The request, which makes the repair idempotent.
    pub(crate) request_id: Uuid,
    /// The archive account the rows belong to.
    pub(crate) account_id: Uuid,
    /// The Platform owner of every fact.
    pub(crate) owner: TenantRef,
    /// The instant of the archive tombstones; every fact is stamped after it.
    pub(crate) tombstoned_at: WireTimestamp,
}

/// Finds the survivors of a deletion. It must run BEFORE the scope's rows are removed: the latest
/// fact of a survivor is an outbox row that the removal takes with the deleted export.
pub(crate) async fn find_survivors(
    transaction: &mut Tx<'_>,
    request_id: Uuid,
    archives: &[Uuid],
) -> Result<Survivors, ProjectionError> {
    Ok(Survivors {
        projects: retained_under_deleted(
            transaction,
            request_id,
            "project",
            &PROJECT_TYPES,
            archives,
        )
        .await?,
        conversations: retained_under_deleted(
            transaction,
            request_id,
            "conversation",
            &CONVERSATION_TYPES,
            archives,
        )
        .await?,
    })
}

/// The planned `retain_evidenced` records of one category whose latest fact names one of
/// `archives`.
async fn retained_under_deleted(
    transaction: &mut Tx<'_>,
    request_id: Uuid,
    category: &str,
    event_types: &[&str; 2],
    archives: &[Uuid],
) -> Result<Vec<Uuid>, ProjectionError> {
    let retained: Vec<String> = sqlx::query_scalar(
        "SELECT opaque_id FROM chatgpt_archive.privacy_deletion_items
         WHERE request_id = $1 AND category = $2 AND action = 'retain_evidenced'
         ORDER BY opaque_id",
    )
    .bind(request_id)
    .bind(category)
    .fetch_all(&mut **transaction)
    .await?;
    let types: Vec<String> = event_types
        .iter()
        .map(|value| (*value).to_owned())
        .collect();
    let mut survivors = Vec::new();
    for opaque_id in retained {
        let id = Uuid::parse_str(&opaque_id).map_err(|_| ProjectionError::Unrepresentable)?;
        let latest: Option<Option<String>> = sqlx::query_scalar(
            "SELECT payload #>> '{payload,import_provenance,ai_archive_id}'
             FROM chatgpt_archive.outbox_events
             WHERE aggregate_id = $1 AND event_type = ANY($2) ORDER BY id DESC LIMIT 1",
        )
        .bind(id)
        .bind(&types)
        .fetch_optional(&mut **transaction)
        .await?;
        let names_deleted = latest
            .flatten()
            .and_then(|archive| Uuid::parse_str(&archive).ok())
            .is_some_and(|archive| archives.contains(&archive));
        if names_deleted {
            survivors.push(id);
        }
    }
    Ok(survivors)
}

/// What a fact needs to name the retained export it is announced under.
struct RetainedImport {
    export_id: Uuid,
    archive_id: Uuid,
    source_export: BlobRef,
    imported_at: WireTimestamp,
    parser: Option<(ParserName, ParserVersion)>,
}

impl RetainedImport {
    async fn load(
        transaction: &mut Tx<'_>,
        export_id: Uuid,
        deletion: &Deletion,
    ) -> Result<Self, ProjectionError> {
        let (archive_id, blob_ref): (Uuid, serde_json::Value) = sqlx::query_as(
            "SELECT ai_archive_id, blob_ref FROM chatgpt_archive.exports WHERE id = $1",
        )
        .bind(export_id)
        .fetch_one(&mut **transaction)
        .await?;
        // The instant the import head announced, so the fact agrees with the head it points at.
        let announced: Option<Option<String>> = sqlx::query_scalar(
            "SELECT payload #>> '{payload,imported_at}' FROM chatgpt_archive.outbox_events
             WHERE event_type = $1 AND aggregate_id = $2 ORDER BY id LIMIT 1",
        )
        .bind(ARCHIVE_IMPORTED)
        .bind(archive_id)
        .fetch_optional(&mut **transaction)
        .await?;
        let run: Option<(String, String)> = sqlx::query_as(
            "SELECT parser_name, parser_version FROM chatgpt_archive.import_runs
             WHERE export_id = $1 AND parser_name IS NOT NULL AND parser_version IS NOT NULL
             ORDER BY started_at DESC, id DESC LIMIT 1",
        )
        .bind(export_id)
        .fetch_optional(&mut **transaction)
        .await?;
        let parser = run
            .map(|(name, version)| -> Result<_, ProjectionError> {
                Ok((
                    contract_parser_name(&name)?,
                    ParserVersion::parse(&version).map_err(|_| ProjectionError::Unrepresentable)?,
                ))
            })
            .transpose()?;
        Ok(Self {
            export_id,
            archive_id,
            source_export: serde_json::from_value(blob_ref)?,
            imported_at: announced
                .flatten()
                .and_then(|value| WireTimestamp::parse(&value).ok())
                .unwrap_or(deletion.tombstoned_at),
            parser,
        })
    }

    /// The provenance a fact is announced with. The parser that last wrote the record's rows
    /// stamps it when there is one; the retained import's own parser stands in otherwise.
    fn base(
        &self,
        deletion: &Deletion,
        stamp: Option<(&str, &str)>,
    ) -> Result<AiArchiveProvenance, ProjectionError> {
        let (parser_name, parser_version) = match (stamp, &self.parser) {
            (Some((name, version)), _) => (
                contract_parser_name(name)?,
                ParserVersion::parse(version).map_err(|_| ProjectionError::Unrepresentable)?,
            ),
            (None, Some((name, version))) => (name.clone(), version.clone()),
            (None, None) => return Err(ProjectionError::Unrepresentable),
        };
        Ok(AiArchiveProvenance {
            ai_archive_id: ratatoskr_identifiers::AiArchiveId(self.archive_id),
            provider: AiProvider::parse("chatgpt").map_err(|_| ProjectionError::Unrepresentable)?,
            owner: deletion.owner,
            source_export: self.source_export.clone(),
            imported_at: self.imported_at,
            parser_name,
            parser_version,
            extensions: Extensions::new(),
        })
    }
}

/// The earliest retained export (by `received_at`, then id) that presented the content the record
/// holds now. An export that presented older content is evidence of that content, not of this, so
/// it is named only when no retained export presented the current content at all.
async fn earliest_retained(
    transaction: &mut Tx<'_>,
    kind: &str,
    id: Uuid,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT o.export_id FROM chatgpt_archive.export_entity_observations o
         JOIN chatgpt_archive.exports e ON e.id = o.export_id
         WHERE o.entity_kind = $1 AND o.entity_id = $2
         ORDER BY o.content_digest IS DISTINCT FROM
                    (CASE WHEN $1 = 'project'
                          THEN (SELECT content_digest FROM chatgpt_archive.projects WHERE id = $2)
                          ELSE (SELECT content_digest FROM chatgpt_archive.conversations
                                WHERE id = $2) END),
                  e.received_at, e.id
         LIMIT 1",
    )
    .bind(kind)
    .bind(id)
    .fetch_optional(&mut **transaction)
    .await
}

/// Announces again every survivor, projects first and then conversations, the order an import
/// uses. It runs in the row-removal transaction after the rows are gone, so a survivor is read as
/// the rows now stand and only retained exports are candidates for its provenance.
pub(crate) async fn reannounce(
    transaction: &mut Tx<'_>,
    deletion: &Deletion,
    survivors: &Survivors,
) -> Result<(), ProjectionError> {
    let mut ordinal = 0_i32;
    let rows: Vec<ProjectRow> = sqlx::query_as(&format!(
        "{PROJECT_SELECT} WHERE p.id = ANY($1) ORDER BY p.external_id, p.id"
    ))
    .bind(&survivors.projects)
    .fetch_all(&mut **transaction)
    .await?;
    for row in rows {
        let id = row.0;
        let stamp = row.5.clone().zip(row.6.clone());
        let Some(export_id) = earliest_retained(transaction, "project", id).await? else {
            continue;
        };
        let import = RetainedImport::load(transaction, export_id, deletion).await?;
        let base = import.base(
            deletion,
            stamp.as_ref().map(|(n, v)| (n.as_str(), v.as_str())),
        )?;
        let ProjectFact {
            project,
            provenance,
            digest,
        } = project_fact(row, &base)?;
        let event = NormalizedArchiveEvent::project_updated(&AiProjectUpdated {
            import_provenance: provenance,
            project,
            content_digest: digest,
            extensions: Extensions::new(),
        })?;
        enqueue(transaction, deletion, &import, event, &mut ordinal).await?;
    }
    for row in messages::conversation_rows_of(transaction, &survivors.conversations).await? {
        let id = row.id;
        let Some(export_id) = earliest_retained(transaction, "conversation", id).await? else {
            continue;
        };
        let import = RetainedImport::load(transaction, export_id, deletion).await?;
        let stamp = row.parser_name.clone().zip(row.parser_version.clone());
        let base = import.base(
            deletion,
            stamp.as_ref().map(|(n, v)| (n.as_str(), v.as_str())),
        )?;
        let (provenance, conversation) = conversation_fact(transaction, row, &base).await?;
        let event = NormalizedArchiveEvent::conversation_updated(&AiConversationUpdated {
            import_provenance: provenance,
            conversation,
            extensions: Extensions::new(),
        })?;
        enqueue(transaction, deletion, &import, event, &mut ordinal).await?;
    }
    Ok(())
}

/// Queues one repair fact, a millisecond after the tombstone plus `ordinal` microseconds so the
/// facts of one deletion keep their order.
async fn enqueue(
    transaction: &mut Tx<'_>,
    deletion: &Deletion,
    import: &RetainedImport,
    event: NormalizedArchiveEvent,
    ordinal: &mut i32,
) -> Result<(), ProjectionError> {
    let rendered: String = sqlx::query_scalar(&format!(
        "SELECT {}",
        pg_instant_sql(
            "($1::timestamptz + interval '1 millisecond' + $2::int * interval '1 microsecond')"
        )
    ))
    .bind(deletion.tombstoned_at.to_wire())
    .bind(*ordinal)
    .fetch_one(&mut **transaction)
    .await?;
    *ordinal += 1;
    let occurred_at = wire_instant(&rendered).ok_or(ProjectionError::Unrepresentable)?;
    let aggregate = event.aggregate_id;
    let event_type = event.event_type;
    let event = event.occurring_at(occurred_at);
    let placement = OutboxPlacement {
        account_id: Some(deletion.account_id),
        // Bound to the export it names, so deleting that export takes the fact with it.
        export_id: Some(import.export_id),
        deduplication_key: Some(format!(
            "reannounce:{}:{event_type}:{aggregate}",
            deletion.request_id
        )),
    };
    crate::Database::enqueue_normalized_event(transaction, &event, &placement).await?;
    Ok(())
}
