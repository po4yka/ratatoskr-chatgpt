//! Synthetic project, Canvas, and asset evidence contract tests.
#![expect(
    clippy::expect_used,
    reason = "integration-test assertions need contextual failure messages"
)]
#![expect(
    clippy::panic,
    reason = "the registry-selection helper must fail explicitly in a contract test"
)]

use std::collections::BTreeSet;

use bytes::Bytes;
use ratatoskr_chatgpt_archive::{
    AcquisitionMode, ArchiveInventory, ArtifactProvenance, AssetAvailability, BlobStore,
    ExtractedArtifact, ParserRegistry, ParserSelection, SyntheticArchiveInput,
    SyntheticConversationsParser,
};

const CONVERSATIONS: &[u8] = include_bytes!("fixtures/synthetic_archive_conversations.json");
const PROJECTS: &[u8] = include_bytes!("fixtures/synthetic_archive_projects.json");
const CANVAS: &[u8] = include_bytes!("fixtures/synthetic_archive_canvas.json");
const ASSETS: &[u8] = include_bytes!("fixtures/synthetic_archive_assets.json");
const NOTE: &[u8] = include_bytes!("fixtures/assets/note.txt");

fn selected_parser() -> ratatoskr_chatgpt_archive::ParserId {
    let mut registry = ParserRegistry::default();
    registry
        .register(SyntheticConversationsParser::registration())
        .expect("synthetic parser registration must be unique");
    let inventory = ArchiveInventory {
        entries: Vec::new(),
        compressed_bytes: 0,
        decompressed_bytes: 0,
        signals: BTreeSet::from(["conversations.json".to_owned()]),
    };
    match registry.select(&inventory, AcquisitionMode::ConsumerExport) {
        ParserSelection::Selected(parser) => parser,
        other => panic!("synthetic parser must be selected, got {other:?}"),
    }
}

async fn extracted_note(store: &BlobStore) -> Vec<ExtractedArtifact> {
    let blob = store
        .store(
            "text/plain",
            futures_util::stream::iter(vec![Ok::<Bytes, std::io::Error>(Bytes::from_static(NOTE))]),
        )
        .await
        .expect("fixture asset must store");
    vec![ExtractedArtifact {
        blob,
        provenance: ArtifactProvenance {
            raw_archive_digest: "fixture-archive".to_owned(),
            entry_path: "assets/note.txt".to_owned(),
        },
        quarantined: false,
    }]
}

#[tokio::test]
async fn project_and_instruction_evidence_is_preserved() {
    let store_root = tempfile::tempdir().expect("temporary blob root");
    let store = BlobStore::new(store_root.path()).expect("blob store");
    let selected = selected_parser();
    let artifacts = Vec::new();
    let input = SyntheticArchiveInput::new(
        &selected,
        CONVERSATIONS,
        Some(PROJECTS),
        Some(CANVAS),
        None,
        &artifacts,
        &store,
    );

    let parsed = SyntheticConversationsParser
        .parse_archive(&input)
        .await
        .expect("synthetic archive fixture must parse");

    assert_eq!(parsed.projects.len(), 1);
    let project = &parsed.projects[0];
    assert_eq!(project.external_id, "project-alpha");
    assert_eq!(project.instructions.len(), 2);
    assert_eq!(project.conversation_external_ids, ["conversation-project"]);
    assert_eq!(project.asset_external_ids, ["asset-uploaded"]);
    assert!(parsed.raw_records.iter().any(|record| {
        record.path == "/projects/0/future_project_field" && record.payload["preserve"] == true
    }));
}

#[tokio::test]
async fn canvas_document_content_is_preserved_as_evidence() {
    let store_root = tempfile::tempdir().expect("temporary blob root");
    let store = BlobStore::new(store_root.path()).expect("blob store");
    let selected = selected_parser();
    let artifacts = Vec::new();
    let input = SyntheticArchiveInput::new(
        &selected,
        CONVERSATIONS,
        Some(PROJECTS),
        Some(CANVAS),
        None,
        &artifacts,
        &store,
    );

    let parsed = SyntheticConversationsParser
        .parse_archive(&input)
        .await
        .expect("synthetic archive fixture must parse");

    assert_eq!(parsed.canvas_documents.len(), 1);
    let canvas = &parsed.canvas_documents[0];
    assert_eq!(canvas.external_id, "canvas-alpha");
    assert_eq!(canvas.project_external_id.as_deref(), Some("project-alpha"));
    assert_eq!(canvas.content.len(), 2);
    assert_eq!(canvas.content[1]["text"], "never execute");
    assert!(
        parsed.raw_records.iter().any(|record| {
            record.path == "/canvas/0/future_canvas_field" && record.payload == 1
        })
    );
}

#[tokio::test]
async fn asset_digest_mismatch_is_quarantined() {
    let store_root = tempfile::tempdir().expect("temporary blob root");
    let store = BlobStore::new(store_root.path()).expect("blob store");
    let selected = selected_parser();
    let artifacts = extracted_note(&store).await;
    let input = SyntheticArchiveInput::new(
        &selected,
        CONVERSATIONS,
        Some(PROJECTS),
        Some(CANVAS),
        Some(ASSETS),
        &artifacts,
        &store,
    );

    let parsed = SyntheticConversationsParser
        .parse_archive(&input)
        .await
        .expect("synthetic archive fixture must parse");

    assert_eq!(
        parsed.assets.first().map(|asset| asset.availability),
        Some(AssetAvailability::Quarantined)
    );
}

#[tokio::test]
async fn verified_asset_keeps_its_blob_reference() {
    let store_root = tempfile::tempdir().expect("temporary blob root");
    let store = BlobStore::new(store_root.path()).expect("blob store");
    let selected = selected_parser();
    let artifacts = extracted_note(&store).await;
    let source = format!(
        r#"[{{"id":"asset-generated","kind":"generated","project_id":"project-alpha","conversation_id":"conversation-project","display_name":"note.txt","archive_path":"assets/note.txt","media_type":"text/plain","length_bytes":{},"sha256":"{}"}}]"#,
        artifacts[0].blob.length_bytes,
        artifacts[0].blob.digest.hex.as_str()
    );
    let input = SyntheticArchiveInput::new(
        &selected,
        CONVERSATIONS,
        Some(PROJECTS),
        Some(CANVAS),
        Some(source.as_bytes()),
        &artifacts,
        &store,
    );

    let parsed = SyntheticConversationsParser
        .parse_archive(&input)
        .await
        .expect("synthetic archive fixture must parse");

    assert_eq!(
        parsed.assets.first().map(|asset| asset.availability),
        Some(AssetAvailability::Verified)
    );
    assert_eq!(
        parsed.assets.first().and_then(|asset| asset.blob.clone()),
        Some(artifacts[0].blob.clone())
    );
}

#[tokio::test]
async fn reference_only_asset_remains_missing() {
    let store_root = tempfile::tempdir().expect("temporary blob root");
    let store = BlobStore::new(store_root.path()).expect("blob store");
    let selected = selected_parser();
    let artifacts = Vec::new();
    let source = br#"[{"id":"asset-reference","kind":"uploaded","display_name":"remote.txt"}]"#;
    let input = SyntheticArchiveInput::new(
        &selected,
        CONVERSATIONS,
        Some(PROJECTS),
        Some(CANVAS),
        Some(source),
        &artifacts,
        &store,
    );

    let parsed = SyntheticConversationsParser
        .parse_archive(&input)
        .await
        .expect("synthetic archive fixture must parse");

    assert_eq!(
        parsed.assets.first().map(|asset| asset.availability),
        Some(AssetAvailability::Missing)
    );
    assert_eq!(
        parsed.assets.first().and_then(|asset| asset.blob.clone()),
        None
    );
}
