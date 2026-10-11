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

mod common;

use bytes::Bytes;
use ratatoskr_chatgpt_archive::{
    AcquisitionMode, ArchiveInventory, ArtifactProvenance, AssetAvailability, BlobStore,
    ExtractedArtifact, ParserRegistry, ParserSelection, SyntheticArchiveInput,
    SyntheticConversationsParser, SyntheticParserError,
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

// ---- XR-021 CONTRACTS.md R3-05: the registered parser path verifies assets.json ----

/// Runs the executor the registry resolves for `entries`, the way the importer does.
async fn execute_registered(
    entries: &[(&str, &[u8])],
) -> Result<
    ratatoskr_chatgpt_archive::ParsedConversations,
    ratatoskr_chatgpt_archive::ParserExecutionError,
> {
    use ratatoskr_chatgpt_archive::{
        ArchiveInspector, ArchiveLimits, ParserArtifactEvidence, ParserExecutionInput,
    };
    let root = tempfile::tempdir().expect("temporary blob root");
    let store = BlobStore::new(root.path()).expect("blob store");
    let archive = common::zip_of(entries);
    let raw = store
        .store(
            "application/zip",
            futures_util::stream::iter(vec![Ok::<Bytes, std::io::Error>(Bytes::from(archive))]),
        )
        .await
        .expect("the raw archive stores");
    let inventory = ArchiveInspector::new(
        store.clone(),
        ArchiveLimits {
            max_entries: 32,
            max_compressed_bytes: 1_048_576,
            max_entry_bytes: 1_048_576,
            max_decompressed_bytes: 2_097_152,
            max_compression_ratio: 100,
        },
    )
    .inspect(&raw)
    .await
    .expect("the archive inspects");
    let registry = ParserRegistry::runtime().expect("the runtime registry builds");
    let selected = match registry.select(&inventory, AcquisitionMode::ConsumerExport) {
        ParserSelection::Selected(parser) => parser,
        other => panic!("the synthetic parser must be selected, got {other:?}"),
    };
    let compiled = registry
        .find_exact(&selected, &inventory, AcquisitionMode::ConsumerExport)
        .expect("the selected parser is compiled");
    let evidence: Vec<ParserArtifactEvidence> = entries
        .iter()
        .map(|(path, bytes)| ParserArtifactEvidence {
            path: (*path).to_owned(),
            bytes: Bytes::copy_from_slice(bytes),
            quarantined: false,
        })
        .collect();
    compiled
        .execute(ParserExecutionInput {
            inventory: &inventory,
            artifacts: &evidence,
            raw_archive: &raw,
            blobs: &store,
        })
        .await
}

fn declaration_of_note() -> Vec<u8> {
    use sha2::Digest as _;
    serde_json::json!([{
        "id": "asset-uploaded",
        "kind": "uploaded",
        "archive_path": "assets/note.txt",
        "media_type": "text/plain",
        "length_bytes": NOTE.len(),
        "sha256": hex::encode(sha2::Sha256::digest(NOTE)),
    }])
    .to_string()
    .into_bytes()
}

#[tokio::test]
async fn an_assets_json_with_every_file_present_imports_the_assets() {
    let parsed = execute_registered(&[
        ("conversations.json", CONVERSATIONS),
        ("assets.json", &declaration_of_note()),
        ("assets/note.txt", NOTE),
    ])
    .await
    .expect("an archive with a declared, present asset imports");

    assert_eq!(parsed.assets.len(), 1);
    assert_eq!(parsed.assets[0].availability, AssetAvailability::Verified);
    let blob = parsed.assets[0]
        .blob
        .as_ref()
        .expect("a verified asset keeps its blob reference");
    assert_eq!(blob.length_bytes, NOTE.len() as u64);
}

#[tokio::test]
async fn a_malformed_assets_json_is_a_parser_failure() {
    let outcome = execute_registered(&[
        ("conversations.json", CONVERSATIONS),
        ("assets.json", b"not json"),
    ])
    .await;

    assert!(outcome.is_err(), "a malformed declaration never imports");
}

/// A storage fault while a declared file is verified is not evidence about the file: it must
/// surface as an error the importer can retry, never as a `BlobVerificationFailed` anomaly that
/// is frozen into a completed or partial import (XR-021 CONTRACTS.md R3-04 rule 2).
#[tokio::test]
async fn a_storage_fault_while_verifying_a_declared_file_is_an_error_not_an_anomaly() {
    use std::os::unix::fs::PermissionsExt as _;

    let store_root = tempfile::tempdir().expect("temporary blob root");
    let store = BlobStore::new(store_root.path()).expect("blob store");
    let selected = selected_parser();
    let artifacts = extracted_note(&store).await;
    let directory = store
        .resolve(&artifacts[0].blob)
        .expect("the stored note resolves")
        .parent()
        .expect("an object lives in a directory")
        .to_path_buf();
    let input = SyntheticArchiveInput::new(
        &selected,
        CONVERSATIONS,
        Some(PROJECTS),
        Some(CANVAS),
        Some(ASSETS),
        &artifacts,
        &store,
    );

    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o000))
        .expect("the object directory can be locked in the test");
    let outcome = SyntheticConversationsParser.parse_archive(&input).await;
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755))
        .expect("the object directory can be unlocked in the test");

    assert_eq!(
        outcome.map(|parsed| parsed.assets.iter().map(|a| a.anomaly).collect::<Vec<_>>()),
        Err(SyntheticParserError::StorageUnavailable),
        "a refused read is not an asset anomaly"
    );
}
