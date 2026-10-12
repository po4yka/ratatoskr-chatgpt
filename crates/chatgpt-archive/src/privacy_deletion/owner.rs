//! Which Platform user a deletion's tombstones are addressed to.
//!
//! Knowledge indexes an archive's derived data under the owner the archive's own announced facts
//! named (XR-021 CONTRACTS.md R4-04). The operator-configured `PLATFORM_ACCOUNTS` mapping is only
//! today's opinion of who owns the account: it can change after an import, and a tombstone
//! addressed from it would then name a tenant Knowledge never indexed the data under. The announced
//! facts are the record of what was told, so they decide, and the mapping answers only for an
//! account that announced nothing.

use ratatoskr_identifiers::TenantRef;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::owner::OwnerResolver;

const ARCHIVE_IMPORTED: &str = "ai_archive.archive.imported.v1";
const TOMBSTONED: &str = "ai_archive.subject.tombstoned.v1";

/// The owner a fact names, from its typed payload and, last, its envelope tenant. The two agree
/// for every fact this service writes; the typed payload is the contract's own statement.
const FACT_OWNER: &str = "COALESCE(
    payload #>> '{payload,owner}',
    payload #>> '{payload,import_provenance,owner}',
    payload #>> '{payload,conversation,owner}',
    payload ->> 'tenant_id')";

/// Where a deletion's tombstones go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TombstoneOwner {
    /// The tombstones are addressed to this Platform user.
    Known(TenantRef),
    /// The account announced no fact and no mapping exists: Knowledge holds nothing to retract.
    NothingAnnounced,
    /// The account announced facts but neither they nor the mapping name an owner.
    Unresolvable,
}

impl TombstoneOwner {
    /// The owner, when one was found.
    pub(super) fn known(self) -> Option<TenantRef> {
        match self {
            Self::Known(owner) => Some(owner),
            Self::NothingAnnounced | Self::Unresolvable => None,
        }
    }
}

/// Resolves the owner of a deletion's tombstones.
///
/// Must run inside the row-removal transaction before the outbox rows of the deleted exports go.
/// The order is: the latest imported head of a deleted archive, then the latest announced fact of
/// the account, then the live mapping.
pub(super) async fn owner_for_deletion(
    transaction: &mut Transaction<'_, Postgres>,
    resolver: &OwnerResolver,
    account_id: Uuid,
    request_id: Uuid,
) -> Result<TombstoneOwner, sqlx::Error> {
    let head_of_deleted_archive = format!(
        "SELECT owner FROM (
           SELECT id, {FACT_OWNER} AS owner FROM chatgpt_archive.outbox_events
           WHERE event_type = $1 AND aggregate_id IN (
             SELECT e.ai_archive_id FROM chatgpt_archive.exports e
             JOIN chatgpt_archive.privacy_deletion_items i ON i.opaque_id = e.id::text
             WHERE i.request_id = $2 AND i.category = 'export' AND i.action = 'remove')
         ) fact WHERE owner LIKE 'user:%' ORDER BY id DESC LIMIT 1"
    );
    let from_archive: Option<String> = sqlx::query_scalar(&head_of_deleted_archive)
        .bind(ARCHIVE_IMPORTED)
        .bind(request_id)
        .fetch_optional(&mut **transaction)
        .await?;
    if let Some(owner) = from_archive.and_then(|raw| TenantRef::parse(&raw).ok()) {
        return Ok(TombstoneOwner::Known(owner));
    }

    let latest_fact_of_account = format!(
        "SELECT owner FROM (
           SELECT id, {FACT_OWNER} AS owner FROM chatgpt_archive.outbox_events
           WHERE tenant_id = $1 AND event_type LIKE 'ai_archive.%' AND event_type <> $2
         ) fact WHERE owner LIKE 'user:%' ORDER BY id DESC LIMIT 1"
    );
    let from_account: Option<String> = sqlx::query_scalar(&latest_fact_of_account)
        .bind(account_id)
        .bind(TOMBSTONED)
        .fetch_optional(&mut **transaction)
        .await?;
    if let Some(owner) = from_account.and_then(|raw| TenantRef::parse(&raw).ok()) {
        return Ok(TombstoneOwner::Known(owner));
    }

    if let Some(owner) = resolver.owner_of_account(transaction, account_id).await? {
        return Ok(TombstoneOwner::Known(owner));
    }
    let announced: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM chatgpt_archive.outbox_events
         WHERE tenant_id = $1 AND event_type LIKE 'ai_archive.%' AND event_type <> $2)",
    )
    .bind(account_id)
    .bind(TOMBSTONED)
    .fetch_one(&mut **transaction)
    .await?;
    Ok(if announced {
        TombstoneOwner::Unresolvable
    } else {
        TombstoneOwner::NothingAnnounced
    })
}
