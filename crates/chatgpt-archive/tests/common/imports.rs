//! A disposable archive account that imports cumulative exports through the real worker under a
//! chosen Platform-user mapping, and deletes through the real service.

use std::sync::Arc;

use ratatoskr_chatgpt_archive::privacy_deletion::{
    DeletionReport, PrivacyDeletionScope, PrivacyDeletionService,
};
use ratatoskr_chatgpt_archive::{BlobStore, InitialImportWorker, ParserRegistry};
use uuid::Uuid;

use super::{DisposableDatabase, Received, archive_limits, drain, receive_export, zip_of};

/// The external reference of the one account every fixture import arrives under.
pub(crate) const ACCOUNT: &str = "acc-owner-ledger";

/// A cumulative export holding the listed `(conversation id, text)` pairs.
pub(crate) fn export(conversations: &[(&str, &str)]) -> Vec<u8> {
    let documents: Vec<serde_json::Value> = conversations
        .iter()
        .map(|(id, text)| {
            serde_json::json!({
                "id": id,
                "title": format!("Title of {id}"),
                "mapping": {"node": {
                    "id": "node",
                    "parent": null,
                    "message": {
                        "id": format!("message-{id}"),
                        "author": {"role": "user"},
                        "content": {"parts": [text]}
                    }
                }}
            })
        })
        .collect();
    let body = serde_json::Value::Array(documents).to_string();
    zip_of(&[("conversations.json", body.as_bytes())])
}

pub(crate) struct Fixture {
    pub(crate) database: DisposableDatabase,
    pub(crate) blobs: BlobStore,
    _root: tempfile::TempDir,
}

impl Fixture {
    pub(crate) async fn new() -> Self {
        let root = tempfile::tempdir().expect("blob root");
        Self {
            database: DisposableDatabase::create().await,
            blobs: BlobStore::new(root.path()).expect("blob store"),
            _root: root,
        }
    }

    /// An import worker whose mapping names `user` for the account, or no mapping at all.
    pub(crate) fn importer(&self, user: Option<&str>) -> InitialImportWorker {
        let mapping = user
            .map(|user| {
                vec![(
                    Uuid::parse_str(user).expect("a literal UUID"),
                    ACCOUNT.to_owned(),
                )]
            })
            .unwrap_or_default();
        InitialImportWorker::new(
            self.database.pool().clone(),
            self.blobs.clone(),
            Arc::new(ParserRegistry::runtime().expect("the runtime registry builds")),
            archive_limits(),
        )
        .with_platform_users(mapping)
    }

    /// Receives and imports one export while the mapping names `user`.
    pub(crate) async fn import(
        &self,
        user: Option<&str>,
        conversations: &[(&str, &str)],
    ) -> Received {
        let received = receive_export(
            self.database.pool(),
            &self.blobs,
            ACCOUNT,
            export(conversations),
        )
        .await;
        drain(&self.importer(user)).await;
        received
    }

    pub(crate) fn service(&self) -> PrivacyDeletionService {
        PrivacyDeletionService::new(self.database.pool().clone(), self.blobs.clone())
    }

    pub(crate) async fn delete(&self, tenant: Uuid, scope: PrivacyDeletionScope) -> DeletionReport {
        let service = self.service();
        let request = Uuid::now_v7();
        service
            .plan(tenant, request, scope)
            .await
            .expect("planning succeeds")
            .expect("the owned scope plans");
        service
            .execute(request)
            .await
            .expect("the deletion completes")
    }

    pub(crate) async fn conversation(&self, external_id: &str) -> Uuid {
        sqlx::query_scalar("SELECT id FROM chatgpt_archive.conversations WHERE external_id = $1")
            .bind(external_id)
            .fetch_one(self.database.pool())
            .await
            .expect("the conversation was projected")
    }
}
