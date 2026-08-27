use std::collections::{BTreeMap, BTreeSet};

use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::{DeletionAction, DeletionInventoryItem, DeletionPlan, PrivacyDeletionScope};
use crate::BlobStore;

/// Privacy deletion planning or execution failure without source content.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PrivacyDeletionError {
    /// Owned persistence failed.
    #[error("privacy deletion persistence failed")]
    Store(#[from] sqlx::Error),
    /// A stable request identity was reused for different arguments or state.
    #[error("privacy deletion request identity conflicts with durable state")]
    Conflict,
    /// Durable inventory contains a value this build cannot interpret.
    #[error("privacy deletion inventory is invalid")]
    InvalidInventory,
    /// Exact blob verification or erasure failed.
    #[error("privacy deletion blob operation failed")]
    Blob(#[from] crate::BlobStoreError),
    /// A content-free report or tombstone could not be encoded.
    #[error("privacy deletion evidence encoding failed")]
    Encode(#[from] serde_json::Error),
    /// A validated tombstone could not be constructed.
    #[error("privacy deletion tombstone construction failed")]
    Tombstone(#[from] crate::OutboxError),
    /// A test seam injected a finalization failure.
    #[error("privacy deletion finalization failed")]
    InjectedFinalization,
}

/// Hand-written persistence fault used to prove transaction ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalizationFault {
    /// Execute without an injected fault.
    None,
    /// Fail after the first selected database row is removed.
    AfterFirstRemoval,
}

/// Coordinates tenant-locked privacy deletion state and exact blob erasure.
#[derive(Debug, Clone)]
pub struct PrivacyDeletionService {
    pub(super) pool: PgPool,
    pub(super) blobs: BlobStore,
}

impl PrivacyDeletionService {
    /// Creates a service over the process-owned pool and `BlobStore`.
    #[must_use]
    pub fn new(pool: PgPool, blobs: BlobStore) -> Self {
        Self { pool, blobs }
    }

    /// Persists a deterministic, content-free preflight inventory.
    ///
    /// `None` deliberately represents both unknown and foreign subjects.
    ///
    /// # Errors
    ///
    /// Returns [`PrivacyDeletionError`] when owned persistence fails or the
    /// request identity contradicts an earlier plan.
    pub async fn plan(
        &self,
        tenant_id: Uuid,
        request_id: Uuid,
        scope: PrivacyDeletionScope,
    ) -> Result<Option<DeletionPlan>, PrivacyDeletionError> {
        let _ = &self.blobs;
        let mut transaction = self.pool.begin().await?;
        lock_tenant(&mut transaction, tenant_id).await?;
        let Some(exports) = selected_exports(&mut transaction, tenant_id, scope).await? else {
            transaction.rollback().await?;
            return Ok(None);
        };
        if let Some(plan) = load_existing(&mut transaction, tenant_id, request_id, scope).await? {
            transaction.commit().await?;
            return Ok(Some(plan));
        }
        let export_ids: Vec<Uuid> = exports.iter().map(|export| export.id).collect();
        let mut inventory = Inventory::default();

        for export in &exports {
            inventory.push("export", export.id, DeletionAction::Remove);
            let blob_action =
                if source_blob_is_shared(&mut transaction, &export_ids, &export.blob_ref).await? {
                    DeletionAction::RetainShared
                } else {
                    DeletionAction::Erase
                };
            inventory.push_blob(
                "raw_archive_blob",
                export.id,
                blob_action,
                export.blob_ref.clone(),
            );
        }
        add_source_rows(&mut transaction, &export_ids, &mut inventory).await?;
        add_normalized_rows(
            &mut transaction,
            &export_ids,
            tenant_id,
            scope,
            &mut inventory,
        )
        .await?;
        add_delivery_rows(
            &mut transaction,
            &export_ids,
            tenant_id,
            scope,
            &mut inventory,
        )
        .await?;
        add_archive_tombstones(&exports, &mut inventory);

        let plan = DeletionPlan::new(request_id, tenant_id, scope, inventory.items);
        persist_plan(
            &mut transaction,
            &plan,
            &inventory.blob_refs,
            scope_parts(scope),
        )
        .await?;
        transaction.commit().await?;
        Ok(Some(plan))
    }
}

#[derive(Debug, Clone)]
struct SelectedExport {
    id: Uuid,
    ai_archive_id: Uuid,
    blob_ref: serde_json::Value,
}

#[derive(Default)]
struct Inventory {
    items: Vec<DeletionInventoryItem>,
    blob_refs: BTreeMap<(String, String), serde_json::Value>,
}

impl Inventory {
    #[expect(
        clippy::needless_pass_by_value,
        reason = "small identifiers become owned immutable inventory strings here"
    )]
    fn push(&mut self, category: &str, id: impl ToString, action: DeletionAction) {
        self.items.push(DeletionInventoryItem {
            category: category.to_owned(),
            opaque_id: id.to_string(),
            action,
        });
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "small identifiers become owned immutable inventory strings here"
    )]
    fn push_blob(
        &mut self,
        category: &str,
        id: impl ToString,
        action: DeletionAction,
        blob_ref: serde_json::Value,
    ) {
        let opaque_id = id.to_string();
        self.blob_refs
            .insert((category.to_owned(), opaque_id.clone()), blob_ref);
        self.items.push(DeletionInventoryItem {
            category: category.to_owned(),
            opaque_id,
            action,
        });
    }
}

pub(crate) async fn lock_tenant(
    transaction: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(tenant_id)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn selected_exports(
    transaction: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    scope: PrivacyDeletionScope,
) -> Result<Option<Vec<SelectedExport>>, sqlx::Error> {
    let tenant_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM chatgpt_archive.accounts WHERE id = $1)")
            .bind(tenant_id)
            .fetch_one(&mut **transaction)
            .await?;
    if !tenant_exists {
        return Ok(None);
    }
    let rows: Vec<(Uuid, Uuid, serde_json::Value)> = match scope {
        PrivacyDeletionScope::Archive { ai_archive_id } => {
            sqlx::query_as(
                "SELECT id, ai_archive_id, blob_ref FROM chatgpt_archive.exports
                 WHERE account_id = $1 AND ai_archive_id = $2 ORDER BY id",
            )
            .bind(tenant_id)
            .bind(ai_archive_id)
            .fetch_all(&mut **transaction)
            .await?
        }
        PrivacyDeletionScope::Conversation { conversation_id } => {
            let owned: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM chatgpt_archive.conversations
                 WHERE id = $1 AND account_id = $2)",
            )
            .bind(conversation_id)
            .bind(tenant_id)
            .fetch_one(&mut **transaction)
            .await?;
            if !owned {
                return Ok(None);
            }
            sqlx::query_as(
                "SELECT DISTINCT e.id, e.ai_archive_id, e.blob_ref
                 FROM chatgpt_archive.exports e
                 JOIN chatgpt_archive.export_entity_observations o ON o.export_id = e.id
                 WHERE e.account_id = $1 AND o.entity_kind = 'conversation'
                   AND o.entity_id = $2 ORDER BY e.id",
            )
            .bind(tenant_id)
            .bind(conversation_id)
            .fetch_all(&mut **transaction)
            .await?
        }
        PrivacyDeletionScope::Tenant => {
            sqlx::query_as(
                "SELECT id, ai_archive_id, blob_ref FROM chatgpt_archive.exports
                 WHERE account_id = $1 ORDER BY id",
            )
            .bind(tenant_id)
            .fetch_all(&mut **transaction)
            .await?
        }
    };
    if matches!(scope, PrivacyDeletionScope::Archive { .. }) && rows.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        rows.into_iter()
            .map(|(id, ai_archive_id, blob_ref)| SelectedExport {
                id,
                ai_archive_id,
                blob_ref,
            })
            .collect(),
    ))
}

async fn add_source_rows(
    transaction: &mut Transaction<'_, Postgres>,
    export_ids: &[Uuid],
    inventory: &mut Inventory,
) -> Result<(), PrivacyDeletionError> {
    let observations: Vec<(Uuid, String, Uuid)> = sqlx::query_as(
        "SELECT export_id, entity_kind, entity_id
         FROM chatgpt_archive.export_entity_observations
         WHERE export_id = ANY($1) ORDER BY export_id, entity_kind, entity_id",
    )
    .bind(export_ids)
    .fetch_all(&mut **transaction)
    .await?;
    for (export_id, kind, entity_id) in observations {
        inventory.push(
            "export_observation",
            format!("{export_id}:{kind}:{entity_id}"),
            DeletionAction::Remove,
        );
    }

    let artifacts: Vec<(Uuid, serde_json::Value)> = sqlx::query_as(
        "SELECT id, blob_ref FROM chatgpt_archive.extracted_artifacts
         WHERE export_id = ANY($1) ORDER BY id",
    )
    .bind(export_ids)
    .fetch_all(&mut **transaction)
    .await?;
    for (id, blob_ref) in artifacts {
        inventory.push("extracted_artifact", id, DeletionAction::Remove);
        let blob_action = if source_blob_is_shared(transaction, export_ids, &blob_ref).await? {
            DeletionAction::RetainShared
        } else {
            DeletionAction::Erase
        };
        inventory.push_blob("extracted_artifact_blob", id, blob_action, blob_ref);
    }
    for id in uuid_rows(
        transaction,
        "SELECT id FROM chatgpt_archive.import_runs WHERE export_id = ANY($1) ORDER BY id",
        export_ids,
    )
    .await?
    {
        inventory.push("import_run", id, DeletionAction::Remove);
    }
    let report_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT c.id FROM chatgpt_archive.completeness_reports c
         JOIN chatgpt_archive.import_runs r ON r.id = c.import_run_id
         WHERE r.export_id = ANY($1) ORDER BY c.id",
    )
    .bind(export_ids)
    .fetch_all(&mut **transaction)
    .await?;
    for id in report_ids {
        inventory.push("completeness_report", id, DeletionAction::Remove);
    }
    for id in uuid_rows(
        transaction,
        "SELECT id FROM chatgpt_archive.raw_records WHERE export_id = ANY($1) ORDER BY id",
        export_ids,
    )
    .await?
    {
        inventory.push("raw_record", id, DeletionAction::Remove);
    }
    Ok(())
}

async fn add_normalized_rows(
    transaction: &mut Transaction<'_, Postgres>,
    export_ids: &[Uuid],
    tenant_id: Uuid,
    scope: PrivacyDeletionScope,
    inventory: &mut Inventory,
) -> Result<(), PrivacyDeletionError> {
    let project_ids = entity_ids(transaction, export_ids, tenant_id, scope, "project").await?;
    let conversation_ids =
        entity_ids(transaction, export_ids, tenant_id, scope, "conversation").await?;
    let mut message_ids = entity_ids(transaction, export_ids, tenant_id, scope, "message").await?;
    let child_messages: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM chatgpt_archive.messages
         WHERE conversation_id = ANY($1) ORDER BY id",
    )
    .bind(&conversation_ids)
    .fetch_all(&mut **transaction)
    .await?;
    message_ids.extend(child_messages);
    message_ids.sort_unstable();
    message_ids.dedup();
    let asset_ids = entity_ids(transaction, export_ids, tenant_id, scope, "asset").await?;

    let actions = add_entity_rows(
        transaction,
        export_ids,
        &project_ids,
        &conversation_ids,
        &message_ids,
        &asset_ids,
        inventory,
    )
    .await?;
    add_message_relations(transaction, export_ids, &message_ids, inventory).await?;
    add_content_parts(transaction, &message_ids, &actions, inventory).await?;
    add_asset_blobs(transaction, &asset_ids, &actions, inventory).await?;
    add_revision_and_tombstones(
        transaction,
        export_ids,
        tenant_id,
        scope,
        &project_ids,
        &conversation_ids,
        &actions,
        inventory,
    )
    .await
}

async fn add_entity_rows(
    transaction: &mut Transaction<'_, Postgres>,
    export_ids: &[Uuid],
    project_ids: &[Uuid],
    conversation_ids: &[Uuid],
    message_ids: &[Uuid],
    asset_ids: &[Uuid],
    inventory: &mut Inventory,
) -> Result<BTreeMap<Uuid, DeletionAction>, PrivacyDeletionError> {
    let mut actions = BTreeMap::new();
    for (category, ids) in [
        ("project", project_ids),
        ("conversation", conversation_ids),
        ("message", message_ids),
        ("asset", asset_ids),
    ] {
        for id in ids {
            let action = if retained_observation(transaction, export_ids, category, *id).await? {
                DeletionAction::RetainEvidenced
            } else {
                DeletionAction::Remove
            };
            actions.insert(*id, action);
            inventory.push(category, id, action);
        }
    }
    Ok(actions)
}

async fn add_message_relations(
    transaction: &mut Transaction<'_, Postgres>,
    export_ids: &[Uuid],
    message_ids: &[Uuid],
    inventory: &mut Inventory,
) -> Result<(), PrivacyDeletionError> {
    let relations: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM chatgpt_archive.message_relations
         WHERE from_message_id = ANY($1) OR to_message_id = ANY($1)
            OR observed_in_export = ANY($2) ORDER BY id",
    )
    .bind(message_ids)
    .bind(export_ids)
    .fetch_all(&mut **transaction)
    .await?;
    for id in relations {
        inventory.push("message_relation", id, DeletionAction::Remove);
    }
    Ok(())
}

async fn add_content_parts(
    transaction: &mut Transaction<'_, Postgres>,
    message_ids: &[Uuid],
    actions: &BTreeMap<Uuid, DeletionAction>,
    inventory: &mut Inventory,
) -> Result<(), PrivacyDeletionError> {
    let parts: Vec<(Uuid, Uuid, Option<serde_json::Value>)> = sqlx::query_as(
        "SELECT id, message_id, blob_ref FROM chatgpt_archive.content_parts
         WHERE message_id = ANY($1) ORDER BY id",
    )
    .bind(message_ids)
    .fetch_all(&mut **transaction)
    .await?;
    for (id, message_id, blob_ref) in parts {
        let action = actions
            .get(&message_id)
            .copied()
            .unwrap_or(DeletionAction::Remove);
        inventory.push("content_part", id, action);
        if let Some(blob_ref) = blob_ref {
            let blob_action = if action == DeletionAction::RetainEvidenced {
                DeletionAction::RetainShared
            } else {
                DeletionAction::Erase
            };
            inventory.push_blob("content_part_blob", id, blob_action, blob_ref);
        }
    }
    Ok(())
}

async fn add_asset_blobs(
    transaction: &mut Transaction<'_, Postgres>,
    asset_ids: &[Uuid],
    actions: &BTreeMap<Uuid, DeletionAction>,
    inventory: &mut Inventory,
) -> Result<(), PrivacyDeletionError> {
    let asset_blobs: Vec<(Uuid, serde_json::Value)> = sqlx::query_as(
        "SELECT id, blob_ref FROM chatgpt_archive.assets
         WHERE id = ANY($1) AND blob_ref IS NOT NULL ORDER BY id",
    )
    .bind(asset_ids)
    .fetch_all(&mut **transaction)
    .await?;
    for (id, blob_ref) in asset_blobs {
        let blob_action = if actions.get(&id) == Some(&DeletionAction::RetainEvidenced) {
            DeletionAction::RetainShared
        } else {
            DeletionAction::Erase
        };
        inventory.push_blob("asset_blob", id, blob_action, blob_ref);
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "the helper owns one explicit ordered deletion-closure boundary"
)]
async fn add_revision_and_tombstones(
    transaction: &mut Transaction<'_, Postgres>,
    export_ids: &[Uuid],
    tenant_id: Uuid,
    scope: PrivacyDeletionScope,
    project_ids: &[Uuid],
    conversation_ids: &[Uuid],
    actions: &BTreeMap<Uuid, DeletionAction>,
    inventory: &mut Inventory,
) -> Result<(), PrivacyDeletionError> {
    for id in uuid_rows(
        transaction,
        "SELECT id FROM chatgpt_archive.revisions WHERE observed_in = ANY($1) ORDER BY id",
        export_ids,
    )
    .await?
    {
        inventory.push("revision", id, DeletionAction::Remove);
    }
    if matches!(scope, PrivacyDeletionScope::Tenant) {
        inventory.push("account", tenant_id, DeletionAction::Remove);
    }
    for id in project_ids {
        if actions.get(id) == Some(&DeletionAction::Remove) {
            inventory.push(
                "downstream_tombstone",
                format!("project:{id}"),
                DeletionAction::EmitTombstone,
            );
        }
    }
    for id in conversation_ids {
        if actions.get(id) == Some(&DeletionAction::Remove) {
            inventory.push(
                "downstream_tombstone",
                format!("conversation:{id}"),
                DeletionAction::EmitTombstone,
            );
        }
    }
    Ok(())
}

async fn add_delivery_rows(
    transaction: &mut Transaction<'_, Postgres>,
    export_ids: &[Uuid],
    tenant_id: Uuid,
    scope: PrivacyDeletionScope,
    inventory: &mut Inventory,
) -> Result<(), PrivacyDeletionError> {
    let all_tenant = matches!(scope, PrivacyDeletionScope::Tenant);
    let inbox: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM chatgpt_archive.inbox_events
         WHERE tenant_id = $1 AND ($2 OR export_id = ANY($3)) ORDER BY id",
    )
    .bind(tenant_id)
    .bind(all_tenant)
    .bind(export_ids)
    .fetch_all(&mut **transaction)
    .await?;
    for id in inbox {
        inventory.push("inbox_event", id, DeletionAction::Remove);
    }
    let outbox: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM chatgpt_archive.outbox_events
         WHERE tenant_id = $1 AND ($2 OR export_id = ANY($3)) ORDER BY id",
    )
    .bind(tenant_id)
    .bind(all_tenant)
    .bind(export_ids)
    .fetch_all(&mut **transaction)
    .await?;
    for id in outbox {
        inventory.push("outbox_event", id, DeletionAction::Remove);
    }
    Ok(())
}

fn add_archive_tombstones(exports: &[SelectedExport], inventory: &mut Inventory) {
    for export in exports {
        inventory.push(
            "downstream_tombstone",
            format!("archive:{}", export.ai_archive_id),
            DeletionAction::EmitTombstone,
        );
    }
}

async fn entity_ids(
    transaction: &mut Transaction<'_, Postgres>,
    export_ids: &[Uuid],
    tenant_id: Uuid,
    scope: PrivacyDeletionScope,
    kind: &str,
) -> Result<Vec<Uuid>, sqlx::Error> {
    let mut ids: BTreeSet<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT entity_id FROM chatgpt_archive.export_entity_observations
         WHERE export_id = ANY($1) AND entity_kind = $2 ORDER BY entity_id",
    )
    .bind(export_ids)
    .bind(kind)
    .fetch_all(&mut **transaction)
    .await?
    .into_iter()
    .collect();
    if let PrivacyDeletionScope::Conversation { conversation_id } = scope
        && kind == "conversation"
    {
        ids.insert(conversation_id);
    }
    if matches!(scope, PrivacyDeletionScope::Tenant) {
        let owned: Vec<Uuid> = match kind {
            "project" => {
                sqlx::query_scalar(
                    "SELECT id FROM chatgpt_archive.projects WHERE account_id = $1 ORDER BY id",
                )
                .bind(tenant_id)
                .fetch_all(&mut **transaction)
                .await?
            }
            "conversation" => sqlx::query_scalar(
                "SELECT id FROM chatgpt_archive.conversations WHERE account_id = $1 ORDER BY id",
            )
            .bind(tenant_id)
            .fetch_all(&mut **transaction)
            .await?,
            "message" => {
                sqlx::query_scalar(
                    "SELECT m.id FROM chatgpt_archive.messages m
                 JOIN chatgpt_archive.conversations c ON c.id = m.conversation_id
                 WHERE c.account_id = $1 ORDER BY m.id",
                )
                .bind(tenant_id)
                .fetch_all(&mut **transaction)
                .await?
            }
            "asset" => {
                sqlx::query_scalar(
                    "SELECT DISTINCT a.id FROM chatgpt_archive.assets a
                 JOIN chatgpt_archive.exports e ON e.id = a.observed_in
                 WHERE e.account_id = $1 ORDER BY a.id",
                )
                .bind(tenant_id)
                .fetch_all(&mut **transaction)
                .await?
            }
            _ => Vec::new(),
        };
        ids.extend(owned);
    }
    Ok(ids.into_iter().collect())
}

async fn retained_observation(
    transaction: &mut Transaction<'_, Postgres>,
    selected_exports: &[Uuid],
    kind: &str,
    entity_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1 FROM chatgpt_archive.export_entity_observations
           WHERE entity_kind = $1 AND entity_id = $2
             AND NOT (export_id = ANY($3))
         )",
    )
    .bind(kind)
    .bind(entity_id)
    .bind(selected_exports)
    .fetch_one(&mut **transaction)
    .await
}

async fn source_blob_is_shared(
    transaction: &mut Transaction<'_, Postgres>,
    selected_exports: &[Uuid],
    blob_ref: &serde_json::Value,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1 FROM chatgpt_archive.exports
           WHERE NOT (id = ANY($1)) AND blob_ref = $2
           UNION ALL
           SELECT 1 FROM chatgpt_archive.extracted_artifacts
           WHERE NOT (export_id = ANY($1)) AND blob_ref = $2
         )",
    )
    .bind(selected_exports)
    .bind(blob_ref)
    .fetch_one(&mut **transaction)
    .await
}

async fn uuid_rows(
    transaction: &mut Transaction<'_, Postgres>,
    query: &str,
    export_ids: &[Uuid],
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar(query)
        .bind(export_ids)
        .fetch_all(&mut **transaction)
        .await
}

fn scope_parts(scope: PrivacyDeletionScope) -> (&'static str, Option<Uuid>) {
    match scope {
        PrivacyDeletionScope::Archive { ai_archive_id } => ("archive", Some(ai_archive_id)),
        PrivacyDeletionScope::Conversation { conversation_id } => {
            ("conversation", Some(conversation_id))
        }
        PrivacyDeletionScope::Tenant => ("tenant", None),
    }
}

async fn persist_plan(
    transaction: &mut Transaction<'_, Postgres>,
    plan: &DeletionPlan,
    blob_refs: &BTreeMap<(String, String), serde_json::Value>,
    (scope_kind, scope_id): (&str, Option<Uuid>),
) -> Result<(), PrivacyDeletionError> {
    sqlx::query(
        "INSERT INTO chatgpt_archive.privacy_deletion_requests
         (id, tenant_id, scope_kind, scope_id, status)
         VALUES ($1, $2, $3, $4, 'planned')",
    )
    .bind(plan.request_id)
    .bind(plan.tenant_id)
    .bind(scope_kind)
    .bind(scope_id)
    .execute(&mut **transaction)
    .await?;
    for (ordinal, item) in plan.items.iter().enumerate() {
        let ordinal = i32::try_from(ordinal).map_err(|_| PrivacyDeletionError::InvalidInventory)?;
        let blob_ref = blob_refs.get(&(item.category.clone(), item.opaque_id.clone()));
        sqlx::query(
            "INSERT INTO chatgpt_archive.privacy_deletion_items
             (request_id, ordinal, category, opaque_id, action, blob_ref)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(plan.request_id)
        .bind(ordinal)
        .bind(&item.category)
        .bind(&item.opaque_id)
        .bind(item.action.as_str())
        .bind(blob_ref)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

async fn load_existing(
    transaction: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    request_id: Uuid,
    scope: PrivacyDeletionScope,
) -> Result<Option<DeletionPlan>, PrivacyDeletionError> {
    let existing: Option<(Uuid, String, Option<Uuid>)> = sqlx::query_as(
        "SELECT tenant_id, scope_kind, scope_id
         FROM chatgpt_archive.privacy_deletion_requests WHERE id = $1",
    )
    .bind(request_id)
    .fetch_optional(&mut **transaction)
    .await?;
    let Some((stored_tenant, stored_kind, stored_scope)) = existing else {
        return Ok(None);
    };
    let (kind, scope_id) = scope_parts(scope);
    if stored_tenant != tenant_id || stored_kind != kind || stored_scope != scope_id {
        return Err(PrivacyDeletionError::Conflict);
    }
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT category, opaque_id, action
         FROM chatgpt_archive.privacy_deletion_items
         WHERE request_id = $1 ORDER BY ordinal",
    )
    .bind(request_id)
    .fetch_all(&mut **transaction)
    .await?;
    let mut items = Vec::with_capacity(rows.len());
    for (category, opaque_id, action) in rows {
        items.push(DeletionInventoryItem {
            category,
            opaque_id,
            action: DeletionAction::parse(&action).ok_or(PrivacyDeletionError::InvalidInventory)?,
        });
    }
    Ok(Some(DeletionPlan::new(request_id, tenant_id, scope, items)))
}
