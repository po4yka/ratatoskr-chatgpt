//! Deterministic portable-archive contract tests.

use std::io::{Cursor, Read as _};

use bytes::Bytes;
use futures_util::stream;
use ratatoskr_chatgpt_archive::BlobStore;
use ratatoskr_chatgpt_archive::portable_export::{
    PortableArchiveExporter, PortableArchiveState, PortableAsset, PortableAssetAvailability,
    PortableConversation, PortableExportFilter, PortableProject, PortableProvenance,
};
use sha2::Digest as _;

fn fixture_state() -> PortableArchiveState {
    PortableArchiveState {
        account_external_ref: "account-alpha".to_owned(),
        provenance: PortableProvenance {
            archive_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_owned(),
            parser_name: "synthetic-conversations".to_owned(),
            parser_version: "1.0.0".to_owned(),
            observed_at_rfc3339: "2026-08-27T00:00:00Z".to_owned(),
        },
        projects: Vec::new(),
        conversations: vec![PortableConversation {
            external_id: "conversation-alpha".to_owned(),
            project_external_id: None,
            title: Some("Portable conversation".to_owned()),
            observed_at_rfc3339: "2026-08-27T00:00:00Z".to_owned(),
            payload: serde_json::json!({"messages": []}),
        }],
        assets: Vec::new(),
    }
}

#[test]
fn identical_state_produces_byte_identical_zip() {
    let exporter = PortableArchiveExporter::new();
    let first = exporter
        .export_to_bytes(&fixture_state())
        .expect("fixture state must export");
    let second = exporter
        .export_to_bytes(&fixture_state())
        .expect("fixture state must export");

    assert!(!first.is_empty(), "portable archive must contain members");
    assert_eq!(
        first, second,
        "identical state must have identical ZIP bytes"
    );
    assert_eq!(
        hex::encode(sha2::Sha256::digest(&first)),
        "ac03b788719be67a00102ce85cb608819f2d5addc63111c8c57a31a1e0b4b716",
        "the stable ZIP layout is a portable output contract"
    );
}

#[test]
fn conversation_markdown_renders_normalized_message_parts() {
    let mut state = fixture_state();
    state.conversations[0].payload = serde_json::json!({
        "messages": [
            {
                "external_id": "message-user",
                "role": "user",
                "parts": [
                    {"ordinal": 0, "part_kind": "text", "payload": {"text": "Keep this text portable."}}
                ]
            }
        ]
    });

    let bytes = PortableArchiveExporter::new()
        .export_to_bytes(&state)
        .expect("fixture state must export");
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("ZIP must be readable");
    let mut markdown = String::new();
    zip.by_name("conversations/conversation-alpha-d45bb53fd24decaa.md")
        .expect("conversation Markdown must be present")
        .read_to_string(&mut markdown)
        .expect("Markdown must be UTF-8");

    assert!(markdown.contains("## user"));
    assert!(markdown.contains("Keep this text portable."));
}

#[tokio::test]
async fn manifest_lists_json_markdown_and_verified_asset_members() {
    let root = tempfile::tempdir().expect("temporary blob root");
    let store = BlobStore::new(root.path()).expect("blob store");
    let blob = store
        .store(
            "text/plain",
            stream::iter([Ok::<Bytes, std::io::Error>(Bytes::from_static(
                b"asset bytes\n",
            ))]),
        )
        .await
        .expect("asset bytes must be stored");
    let mut state = fixture_state();
    state.assets.push(PortableAsset {
        external_id: "asset-alpha".to_owned(),
        project_external_id: None,
        observed_at_rfc3339: "2026-08-27T00:00:00Z".to_owned(),
        availability: PortableAssetAvailability::Verified,
        blob: Some(blob),
        media_type: Some("text/plain".to_owned()),
    });

    let bytes = PortableArchiveExporter::new()
        .export_to_bytes_with_assets(&state, &store)
        .await
        .expect("verified fixture asset must export");
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("ZIP must be readable");
    let mut manifest = String::new();
    zip.by_name("manifest.json")
        .expect("manifest must be present")
        .read_to_string(&mut manifest)
        .expect("manifest must be UTF-8");
    let manifest: serde_json::Value = serde_json::from_str(&manifest).expect("manifest JSON");
    let members = manifest["members"].as_array().expect("member list");
    assert!(members.iter().any(|member| {
        member["path"] == "conversations/conversation-alpha-d45bb53fd24decaa.json"
            && member["sha256"].is_string()
            && member["provenance"]["archive_sha256"]
                == "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    }));
    assert!(members.iter().any(|member| {
        member["path"] == "assets/asset-alpha-6b0162feddc27a6e"
            && member["media_type"] == "text/plain"
    }));
    assert_eq!(
        zip.by_name("assets/asset-alpha-6b0162feddc27a6e")
            .expect("verified asset must be present")
            .size(),
        12
    );
}

#[tokio::test]
async fn unreadable_verified_asset_aborts_without_archive() {
    let root = tempfile::tempdir().expect("temporary blob root");
    let store = BlobStore::new(root.path()).expect("blob store");
    let blob = store
        .store(
            "text/plain",
            stream::iter([Ok::<Bytes, std::io::Error>(Bytes::from_static(
                b"asset bytes\n",
            ))]),
        )
        .await
        .expect("asset bytes must be stored");
    std::fs::remove_file(store.resolve(&blob).expect("stored path"))
        .expect("test may remove its own blob");
    let mut state = fixture_state();
    state.assets.push(PortableAsset {
        external_id: "asset-missing".to_owned(),
        project_external_id: None,
        observed_at_rfc3339: "2026-08-27T00:00:00Z".to_owned(),
        availability: PortableAssetAvailability::Verified,
        blob: Some(blob),
        media_type: Some("text/plain".to_owned()),
    });

    let result = PortableArchiveExporter::new()
        .export_to_bytes_with_assets(&state, &store)
        .await;

    assert!(
        result.is_err(),
        "an unreadable verified asset must abort export"
    );
}

#[test]
fn filters_limit_export_to_matching_project_and_observed_time() {
    let mut alpha = fixture_state();
    alpha.projects.push(PortableProject {
        external_id: "project-alpha".to_owned(),
        title: Some("Alpha project".to_owned()),
        observed_at_rfc3339: "2026-08-27T00:00:00Z".to_owned(),
        payload: serde_json::json!({}),
    });
    alpha.conversations[0].project_external_id = Some("project-alpha".to_owned());
    alpha.conversations.push(PortableConversation {
        external_id: "conversation-before".to_owned(),
        project_external_id: Some("project-alpha".to_owned()),
        title: Some("Before range".to_owned()),
        observed_at_rfc3339: "2026-08-26T23:59:59Z".to_owned(),
        payload: serde_json::json!({}),
    });
    let filter = PortableExportFilter {
        account_external_ref: "account-alpha".to_owned(),
        project_external_id: Some("project-alpha".to_owned()),
        observed_from_rfc3339: Some("2026-08-27T00:00:00Z".to_owned()),
        observed_to_rfc3339: Some("2026-08-27T00:00:00Z".to_owned()),
    };

    let bytes = PortableArchiveExporter::new()
        .export_selected_to_bytes(&[alpha], &filter)
        .expect("matching fixture state must export");
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("ZIP must be readable");
    assert!(
        zip.by_name("conversations/conversation-alpha-d45bb53fd24decaa.json")
            .is_ok()
    );
    assert!(
        zip.by_name("conversations/conversation-before-a18b0ca8d8b18d8d.json")
            .is_err()
    );
    let mut manifest = String::new();
    zip.by_name("manifest.json")
        .expect("manifest must be present")
        .read_to_string(&mut manifest)
        .expect("manifest must be UTF-8");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&manifest).expect("manifest JSON")["filters"]["project_external_id"],
        "project-alpha"
    );
}

#[test]
fn tenant_scope_excludes_other_account_evidence() {
    let alpha = fixture_state();
    let mut beta = fixture_state();
    beta.account_external_ref = "account-beta".to_owned();
    beta.conversations[0].external_id = "conversation-beta".to_owned();
    beta.conversations[0].title = Some("Other account".to_owned());
    let filter = PortableExportFilter {
        account_external_ref: "account-alpha".to_owned(),
        project_external_id: None,
        observed_from_rfc3339: None,
        observed_to_rfc3339: None,
    };

    let bytes = PortableArchiveExporter::new()
        .export_selected_to_bytes(&[beta, alpha], &filter)
        .expect("requested tenant fixture state must export");
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("ZIP must be readable");
    assert!(
        zip.by_name("conversations/conversation-alpha-d45bb53fd24decaa.json")
            .is_ok()
    );
    assert!(
        zip.by_name("conversations/conversation-beta-a5a2b87fea41be24.json")
            .is_err()
    );
}
