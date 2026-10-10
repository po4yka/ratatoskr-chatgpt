//! Binds an archive account to the Platform user that owns its facts.
//!
//! Every `ai_archive.*` fact names its owner as `user:<Platform user>`, which is the
//! tenant Knowledge indexes under (XR-021 CONTRACTS.md section S07). The archive
//! keeps its own account identity internally; the only bridge between the two is the
//! operator-configured `platform_accounts` list of `(Platform user, account external
//! reference)` pairs. The resolver is a constructor parameter of every component that
//! emits a fact, never a global, and it is consulted before an account row can be
//! deleted.

use std::collections::HashMap;

use ratatoskr_identifiers::{TenantRef, UserId};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Reverse lookup from an account's external reference to its Platform owner.
#[derive(Debug, Clone, Default)]
pub struct OwnerResolver {
    by_external_ref: HashMap<String, TenantRef>,
}

impl OwnerResolver {
    /// Builds a resolver from `(Platform user, account external reference)` pairs.
    #[must_use]
    pub fn new(platform_accounts: impl IntoIterator<Item = (Uuid, String)>) -> Self {
        let by_external_ref = platform_accounts
            .into_iter()
            .map(|(user, external_ref)| (external_ref, TenantRef::of_user(UserId(user))))
            .collect();
        Self { by_external_ref }
    }

    /// The Platform owner of an account external reference, when one is configured.
    #[must_use]
    pub fn owner_of(&self, account_external_ref: &str) -> Option<TenantRef> {
        self.by_external_ref.get(account_external_ref).copied()
    }

    /// The Platform owner of an archive account row.
    ///
    /// `None` covers an unmapped account and a missing account alike; neither
    /// emits a fact.
    ///
    /// # Errors
    ///
    /// Returns the database error when the account row cannot be read.
    pub(crate) async fn owner_of_account(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        account_id: Uuid,
    ) -> Result<Option<TenantRef>, sqlx::Error> {
        let external_ref: Option<Option<String>> =
            sqlx::query_scalar("SELECT external_ref FROM chatgpt_archive.accounts WHERE id = $1")
                .bind(account_id)
                .fetch_optional(&mut **transaction)
                .await?;
        Ok(external_ref
            .flatten()
            .and_then(|external_ref| self.owner_of(&external_ref)))
    }
}
