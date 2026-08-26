//! Tenant authentication for archive receipt.
//!
//! A credential authenticates to exactly one owning archive account. Every
//! failure reason renders as the same disclosure-free refusal.

use secrecy::ExposeSecret as _;

use crate::config::ReceiptConfig;

/// The tenant a receipt acts for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantPrincipal {
    /// The account external reference this credential authenticates to.
    pub account_external_ref: String,
}

/// Why a credential was refused. Deliberately carries no detail: missing,
/// malformed, and unknown credentials are indistinguishable to a caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unauthenticated;

impl core::fmt::Display for Unauthenticated {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("authentication is required")
    }
}

impl std::error::Error for Unauthenticated {}

/// Resolves a presented credential to its tenant.
pub trait TenantAuthenticator: core::fmt::Debug + Send + Sync {
    /// Authenticates the raw `Authorization` header value, if one arrived.
    ///
    /// # Errors
    ///
    /// Returns [`Unauthenticated`] for every absent, malformed, or unknown
    /// credential, identically.
    fn authenticate(&self, credential: Option<&str>) -> Result<TenantPrincipal, Unauthenticated>;
}

/// The configured bearer-token map: one token per tenant account.
#[derive(core::fmt::Debug)]
pub struct ConfigTenantAuthenticator {
    entries: std::collections::HashMap<String, String>,
}

impl ConfigTenantAuthenticator {
    /// Builds an authenticator from token-to-account pairs.
    #[must_use]
    pub fn new(pairs: Vec<(String, String)>) -> Self {
        Self {
            entries: pairs.into_iter().collect(),
        }
    }

    /// Builds an authenticator from configured tenant credentials, sealing
    /// each token out of the configuration structure.
    #[must_use]
    pub fn from_config(config: &ReceiptConfig) -> Self {
        let entries = config
            .tenant_tokens
            .iter()
            .map(|pair| {
                (
                    pair.token.expose_secret().to_owned(),
                    pair.account_external_ref.clone(),
                )
            })
            .collect();
        Self { entries }
    }
}

impl TenantAuthenticator for ConfigTenantAuthenticator {
    fn authenticate(&self, credential: Option<&str>) -> Result<TenantPrincipal, Unauthenticated> {
        let Some(header) = credential else {
            return Err(Unauthenticated);
        };
        let Some((scheme, token)) = header.split_once(' ') else {
            return Err(Unauthenticated);
        };
        if !scheme.eq_ignore_ascii_case("bearer") {
            return Err(Unauthenticated);
        }
        if token.is_empty() || token.contains(char::is_whitespace) {
            return Err(Unauthenticated);
        }
        let Some(reference) = self.entries.get(token) else {
            return Err(Unauthenticated);
        };
        Ok(TenantPrincipal {
            account_external_ref: reference.clone(),
        })
    }
}
