//! The persisted import is projected into `ai_archive.*` facts (XR-021 CONTRACTS.md
//! section S07). Every test runs against a disposable `PostgreSQL` database and a
//! synthetic export: the parsers are bound to a synthetic schema, so nothing here
//! claims anything about real `ChatGPT` exports.

// Test bodies fail through `expect`/`panic!`; assertions are the contract.
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test failures report through panics"
)]

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::{DisposableDatabase, archive_limits, drain, receive_export};
use ratatoskr_ai_archive_contracts::{
    AiArchiveImport, AiAuthorRole, AiContentPart, AiConversationAdded, AiConversationUpdated,
    AiProjectAdded,
};
use ratatoskr_chatgpt_archive::reparse::ReparseEngine;
use ratatoskr_chatgpt_archive::{
    AcquisitionMode, BlobStore, InitialImportWorker, ParsedConversations, ParserExecutionError,
    ParserExecutionInput, ParserExecutor, ParserId, ParserRegistration, ParserRegistry,
    SYNTHETIC_PARSER_NAME, SyntheticConversationsParser,
};
use ratatoskr_event_envelope::EventEnvelope;
use uuid::Uuid;

const PLATFORM_USER: &str = "018f0000-0000-7000-8000-000000000005";

fn mapping(account_ref: &str) -> Vec<(Uuid, String)> {
    vec![(
        Uuid::parse_str(PLATFORM_USER).expect("a literal UUID"),
        account_ref.to_owned(),
    )]
}

fn worker(
    database: &DisposableDatabase,
    blobs: &BlobStore,
    mapping: Vec<(Uuid, String)>,
) -> InitialImportWorker {
    InitialImportWorker::new(
        database.pool().clone(),
        blobs.clone(),
        Arc::new(ParserRegistry::runtime().expect("the runtime registry builds")),
        archive_limits(),
    )
    .with_platform_users(mapping)
}

/// Every `ai_archive.*` outbox row, oldest first, as the envelope it stores.
async fn facts(database: &DisposableDatabase) -> Vec<(String, EventEnvelope)> {
    let rows: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT event_type, payload FROM chatgpt_archive.outbox_events
         WHERE event_type LIKE 'ai_archive.%' ORDER BY id",
    )
    .fetch_all(database.pool())
    .await
    .expect("the outbox is readable");
    rows.into_iter()
        .map(|(event_type, payload)| {
            let envelope: EventEnvelope = serde_json::from_value(payload)
                .expect("every stored row is a complete event envelope");
            (event_type, envelope)
        })
        .collect()
}

fn types(facts: &[(String, EventEnvelope)]) -> Vec<&str> {
    facts
        .iter()
        .map(|(event_type, _)| event_type.as_str())
        .collect()
}

#[tokio::test]
async fn an_import_enqueues_the_contract_facts_in_order() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let account_ref = format!("acc-{}", Uuid::now_v7());
    let received =
        receive_export(database.pool(), &blobs, &account_ref, common::project_zip()).await;

    drain(&worker(&database, &blobs, mapping(&account_ref))).await;

    let facts = facts(&database).await;
    assert_eq!(
        types(&facts),
        [
            "ai_archive.archive.imported.v1",
            "ai_archive.project.added.v1",
            "ai_archive.conversation.added.v1",
        ],
        "one head, one project, one conversation, in contract order"
    );
    let owner = format!("user:{PLATFORM_USER}");
    for (_, envelope) in &facts {
        assert_eq!(envelope.producer.as_str(), "ratatoskr-chatgpt");
        assert_eq!(
            envelope.tenant_id.map(|tenant| tenant.to_string()),
            Some(owner.clone()),
            "the owner is the Platform user, never the archive account"
        );
    }

    let import: AiArchiveImport = facts[0].1.payload_as().expect("the head round-trips");
    assert_eq!(import.ai_archive_id.0, received.archive);
    assert_eq!(import.owner.to_string(), owner);
    assert_eq!(import.completeness_report.conversation_count, 1);
    assert_eq!(import.completeness_report.message_count, 1);
    assert!(
        import.completeness_report.gap_count >= 1
            && usize::try_from(import.completeness_report.gap_count)
                == Ok(import.completeness_report.gaps.len()),
        "an incomplete import names its gaps"
    );
    import.validate().expect("the head satisfies invariant A1");

    let project: AiProjectAdded = facts[1].1.payload_as().expect("the project round-trips");
    project.validate().expect("the project provenance agrees");
    assert_eq!(project.project.title.as_str(), "Synthetic project");
    assert_eq!(
        project
            .project
            .instructions
            .as_ref()
            .map(ratatoskr_ai_archive_contracts::AiText::as_str),
        Some("Preserve evidence.\n\nStay inert.")
    );

    let conversation: AiConversationAdded = facts[2]
        .1
        .payload_as()
        .expect("the conversation round-trips");
    conversation
        .validate()
        .expect("provenance agrees and the content digest is recomputed");
    let conversation = conversation.conversation;
    assert_eq!(
        conversation
            .project_ref
            .map(|reference| reference.to_wire()),
        Some(format!("ai_project:{}", project.project.ai_project_id.0)),
        "the conversation points at the persisted project"
    );
    assert_eq!(conversation.messages.len(), 1);
    let message = &conversation.messages[0];
    assert_eq!(message.author_role, AiAuthorRole::User);
    assert!(matches!(message.parts[0], AiContentPart::Text { .. }));
    assert!(
        message.parts[1].is_unknown(),
        "a media reference without verified bytes is preserved as a provider part"
    );
    database.discard().await;
}

#[tokio::test]
async fn an_account_without_a_platform_mapping_emits_no_fact() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let account_ref = format!("acc-{}", Uuid::now_v7());
    receive_export(
        database.pool(),
        &blobs,
        &account_ref,
        common::conversations_only_zip(),
    )
    .await;

    drain(&worker(&database, &blobs, Vec::new())).await;

    assert!(
        facts(&database).await.is_empty(),
        "an unmapped owner would name a tenant Knowledge cannot match"
    );
    let finished: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chatgpt_archive.import_runs WHERE state IN ('completed', 'partial')",
    )
    .fetch_one(database.pool())
    .await
    .expect("runs are readable");
    assert_eq!(finished, 1, "the import itself still completes");
    database.discard().await;
}

#[tokio::test]
async fn internal_messages_are_left_out_with_a_warning_and_children_are_reparented() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let account_ref = format!("acc-{}", Uuid::now_v7());
    let document = serde_json::json!([{
        "id": "chain",
        "title": "Chain",
        "mapping": {
            "a-user": {"id": "a-user", "parent": null, "message": {
                "id": "a-user", "author": {"role": "user"}, "content": {"parts": ["question"]}}},
            "b-internal": {"id": "b-internal", "parent": "a-user", "message": {
                "id": "b-internal", "author": {"role": "internal"}, "content": {"parts": ["hidden"]}}},
            "c-assistant": {"id": "c-assistant", "parent": "b-internal", "message": {
                "id": "c-assistant", "author": {"role": "assistant"}, "content": {"parts": ["answer"]}}}
        }
    }]);
    let archive = common::zip_of(&[("conversations.json", document.to_string().as_bytes())]);
    receive_export(database.pool(), &blobs, &account_ref, archive).await;

    drain(&worker(&database, &blobs, mapping(&account_ref))).await;

    let facts = facts(&database).await;
    let added: AiConversationAdded = facts
        .iter()
        .find(|(event_type, _)| event_type == "ai_archive.conversation.added.v1")
        .expect("a conversation fact exists")
        .1
        .payload_as()
        .expect("the conversation round-trips");
    added
        .validate()
        .expect("the digest covers the represented messages");
    let ids: Vec<&str> = added
        .conversation
        .messages
        .iter()
        .map(|message| message.external_message_id.as_str())
        .collect();
    assert_eq!(ids, ["a-user", "c-assistant"]);
    assert_eq!(
        added.conversation.messages[1]
            .parent_message_id
            .as_ref()
            .map(ratatoskr_identifiers::EntityLocalId::as_str),
        Some("a-user"),
        "the child of a left-out message hangs from the nearest represented ancestor"
    );
    let codes: Vec<&str> = added
        .conversation
        .warnings
        .iter()
        .map(|warning| warning.code.as_str())
        .collect();
    assert_eq!(codes, ["ai_archive.message_role_unrepresentable"]);
    database.discard().await;
}

/// A newer parser release that returns what the synthetic parser returns, optionally
/// with the first message of the first conversation rewritten.
#[derive(Debug)]
struct NewerRelease {
    id: ParserId,
    rewrite_first_message: Option<&'static str>,
}

impl ParserExecutor for NewerRelease {
    fn execute(
        &self,
        input: ParserExecutionInput<'_>,
    ) -> Result<ParsedConversations, ParserExecutionError> {
        let mut parsed = SyntheticConversationsParser.execute(input)?;
        parsed.parser = self.id.clone();
        if let Some(text) = self.rewrite_first_message
            && let Some(part) = parsed
                .conversations
                .first_mut()
                .and_then(|conversation| conversation.messages.first_mut())
                .and_then(|message| message.parts.first_mut())
        {
            part.payload = serde_json::Value::String(text.to_owned());
        }
        Ok(parsed)
    }
}

/// An engine whose registry holds the synthetic parser plus the given newer releases.
fn engine_with_releases(
    database: &DisposableDatabase,
    blobs: &BlobStore,
    releases: &[(&str, Option<&'static str>)],
    mapping: Vec<(Uuid, String)>,
) -> ReparseEngine {
    let mut registry = ParserRegistry::runtime().expect("the runtime registry builds");
    for (version, rewrite_first_message) in releases {
        let id = release_id(version);
        registry
            .register_compiled(
                ParserRegistration {
                    id: id.clone(),
                    modes: vec![AcquisitionMode::ConsumerExport],
                    required_signals: BTreeSet::from(["conversations.json".to_owned()]),
                },
                Arc::new(NewerRelease {
                    id,
                    rewrite_first_message: *rewrite_first_message,
                }),
            )
            .expect("the newer release registers");
    }
    ReparseEngine::new(
        database.pool().clone(),
        blobs.clone(),
        Arc::new(registry),
        archive_limits(),
    )
    .with_platform_users(mapping)
}

fn release_id(version: &str) -> ParserId {
    ParserId {
        name: SYNTHETIC_PARSER_NAME.to_owned(),
        version: version.to_owned(),
    }
}

#[tokio::test]
async fn a_reparse_adds_a_fact_only_for_content_that_changed() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let account_ref = format!("acc-{}", Uuid::now_v7());
    let received = receive_export(
        database.pool(),
        &blobs,
        &account_ref,
        common::single_message_zip("conversation-1", "original"),
    )
    .await;
    drain(&worker(&database, &blobs, mapping(&account_ref))).await;
    let after_import = facts(&database).await;
    assert_eq!(
        types(&after_import),
        [
            "ai_archive.archive.imported.v1",
            "ai_archive.conversation.added.v1"
        ]
    );

    let engine = engine_with_releases(
        &database,
        &blobs,
        &[("0.2.0", None), ("0.3.0", Some("rewritten"))],
        mapping(&account_ref),
    );
    let plan = engine
        .plan(received.account, received.archive, release_id("0.2.0"))
        .await
        .expect("a newer release plans");
    engine.apply(&plan).await.expect("the reparse applies");
    assert_eq!(
        facts(&database).await.len(),
        after_import.len(),
        "identical content adds no row, not even a new import head"
    );

    let plan = engine
        .plan(received.account, received.archive, release_id("0.3.0"))
        .await
        .expect("a newer release plans");
    engine.apply(&plan).await.expect("the reparse applies");

    let after_change = facts(&database).await;
    let updated: Vec<&(String, EventEnvelope)> = after_change
        .iter()
        .filter(|(event_type, _)| event_type == "ai_archive.conversation.updated.v1")
        .collect();
    assert_eq!(updated.len(), 1, "exactly one conversation.updated");
    let updated: AiConversationUpdated = updated[0].1.payload_as().expect("the update round-trips");
    updated
        .validate()
        .expect("the new digest matches the new messages");
    let AiContentPart::Text { text } = &updated.conversation.messages[0].parts[0] else {
        panic!("the rewritten part is text");
    };
    assert_eq!(text.as_str(), "rewritten");
    assert_eq!(
        updated.import_provenance.parser_version.as_str(),
        "0.3.0",
        "the fact names the parser that produced it"
    );
    database.discard().await;
}
