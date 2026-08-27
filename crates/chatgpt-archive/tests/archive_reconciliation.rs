//! Archive reconciliation contract.

#![expect(
    clippy::expect_used,
    reason = "fixture parsing must fail with context in integration tests"
)]
#![expect(
    clippy::panic,
    reason = "fixture parser selection must fail explicitly in contract tests"
)]

use std::collections::BTreeSet;

use bytes::Bytes;
use ratatoskr_chatgpt_archive::{
    AcquisitionMode, ArchiveInventory, ArchiveReconciler, ArchiveSnapshot, ArtifactProvenance,
    BlobStore, Completeness, CoverageGap, ExtractedArtifact, ObservationState, ParserRegistry,
    ParserSelection, SyntheticArchiveInput, SyntheticConversationsParser, WarningCode,
};

const FIXTURE: &[u8] = include_bytes!("fixtures/synthetic_conversations.json");
const ARCHIVE_CONVERSATIONS: &[u8] =
    include_bytes!("fixtures/synthetic_archive_conversations.json");
const ARCHIVE_PROJECTS: &[u8] = include_bytes!("fixtures/synthetic_archive_projects.json");
const ARCHIVE_CANVAS: &[u8] = include_bytes!("fixtures/synthetic_archive_canvas.json");
const ARCHIVE_ASSETS: &[u8] = include_bytes!("fixtures/synthetic_archive_assets.json");
const NOTE: &[u8] = include_bytes!("fixtures/assets/note.txt");

async fn parsed_fixture() -> ratatoskr_chatgpt_archive::ParsedConversations {
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
    let ParserSelection::Selected(parser) =
        registry.select(&inventory, AcquisitionMode::ConsumerExport)
    else {
        panic!("synthetic parser must be selected");
    };
    let root = tempfile::tempdir().expect("temporary blob root");
    let store = BlobStore::new(root.path()).expect("blob store");
    let artifacts = Vec::new();
    let input = SyntheticArchiveInput::new(&parser, FIXTURE, None, None, None, &artifacts, &store);
    SyntheticConversationsParser
        .parse_archive(&input)
        .await
        .expect("synthetic fixture must parse")
}

async fn snapshot(archive_id: &str, sequence: u64) -> ArchiveSnapshot {
    ArchiveSnapshot {
        archive_id: archive_id.to_owned(),
        sequence,
        parsed: parsed_fixture().await,
    }
}

async fn parsed_archive_fixture() -> ratatoskr_chatgpt_archive::ParsedConversations {
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
    let ParserSelection::Selected(selected) =
        registry.select(&inventory, AcquisitionMode::ConsumerExport)
    else {
        panic!("synthetic parser must be selected");
    };
    let root = tempfile::tempdir().expect("temporary blob root");
    let store = BlobStore::new(root.path()).expect("blob store");
    let blob = store
        .store(
            "text/plain",
            futures_util::stream::iter(vec![Ok::<Bytes, std::io::Error>(Bytes::from_static(NOTE))]),
        )
        .await
        .expect("fixture asset must store");
    let artifacts = vec![ExtractedArtifact {
        blob,
        provenance: ArtifactProvenance {
            raw_archive_digest: "fixture-archive".to_owned(),
            entry_path: "assets/note.txt".to_owned(),
        },
        quarantined: false,
    }];
    let input = SyntheticArchiveInput::new(
        &selected,
        ARCHIVE_CONVERSATIONS,
        Some(ARCHIVE_PROJECTS),
        Some(ARCHIVE_CANVAS),
        Some(ARCHIVE_ASSETS),
        &artifacts,
        &store,
    );
    SyntheticConversationsParser
        .parse_archive(&input)
        .await
        .expect("synthetic archive fixture must parse")
}

#[tokio::test]
async fn revision_chain_builds_across_fixture_exports() {
    let first = snapshot("archive-one", 1).await;
    let mut changed = parsed_fixture().await;
    let alpha = changed
        .conversations
        .iter_mut()
        .find(|conversation| conversation.external_id == "conversation-alpha")
        .expect("alpha conversation must be present");
    alpha.title = Some("Revised synthetic archive".to_owned());
    alpha
        .messages
        .iter_mut()
        .find(|message| message.external_id == "message-assistant")
        .expect("assistant message must be present")
        .model_slug = Some("reconciler-test-model".to_owned());
    let second = ArchiveSnapshot {
        archive_id: "archive-two".to_owned(),
        sequence: 2,
        parsed: changed,
    };

    let result = ArchiveReconciler.reconcile(&[first, second]);

    assert_eq!(result.conversations.len(), 2);
    let alpha = result
        .conversations
        .iter()
        .find(|conversation| conversation.external_id == "conversation-alpha")
        .expect("alpha history must exist");
    assert_eq!(alpha.revisions.len(), 2);
    assert_eq!(alpha.observations.len(), 2);
    let assistant = alpha
        .messages
        .iter()
        .find(|message| message.external_id == "message-assistant")
        .expect("assistant history must exist");
    assert_eq!(assistant.revisions.len(), 2);
    let beta = result
        .conversations
        .iter()
        .find(|conversation| conversation.external_id == "conversation-beta")
        .expect("beta history must exist");
    assert_eq!(beta.revisions.len(), 1);
    assert_eq!(beta.observations.len(), 2);
}

#[tokio::test]
async fn missing_conversation_becomes_observation_not_deletion() {
    let first = snapshot("archive-one", 1).await;
    let mut later = parsed_fixture().await;
    later
        .conversations
        .retain(|conversation| conversation.external_id != "conversation-beta");
    let second = ArchiveSnapshot {
        archive_id: "archive-two".to_owned(),
        sequence: 2,
        parsed: later,
    };

    let result = ArchiveReconciler.reconcile(&[first, second]);

    let beta = result
        .conversations
        .iter()
        .find(|conversation| conversation.external_id == "conversation-beta")
        .expect("omitted conversation history must remain");
    assert_eq!(beta.revisions.len(), 1);
    assert_eq!(beta.observations.len(), 2);
    assert_eq!(
        beta.observations[1].state,
        ObservationState::MissingFromLatestSnapshot
    );
    assert_eq!(beta.observations[1].revision_digest, None);
}

#[tokio::test]
async fn orphan_parent_is_retained_and_reported() {
    let mut parsed = parsed_fixture().await;
    let alpha = parsed
        .conversations
        .iter_mut()
        .find(|conversation| conversation.external_id == "conversation-alpha")
        .expect("alpha conversation must be present");
    alpha
        .messages
        .iter_mut()
        .find(|message| message.external_id == "message-user")
        .expect("user message must be present")
        .parent_external_id = Some("not-in-this-conversation".to_owned());
    let result = ArchiveReconciler.reconcile(&[ArchiveSnapshot {
        archive_id: "orphan-archive".to_owned(),
        sequence: 1,
        parsed,
    }]);

    assert_eq!(result.archive_reports.len(), 1);
    let alpha = result
        .conversations
        .iter()
        .find(|conversation| conversation.external_id == "conversation-alpha")
        .expect("alpha history must exist");
    let user = alpha
        .messages
        .iter()
        .find(|message| message.external_id == "message-user")
        .expect("user history must exist");
    assert!(user.observations[0].orphaned);
    assert!(
        result.archive_reports[0]
            .warnings
            .iter()
            .any(|warning| warning.code == WarningCode::MissingParent)
    );
}

#[tokio::test]
async fn conversation_only_snapshot_reports_project_relationship_gap() {
    let result = ArchiveReconciler.reconcile(&[snapshot("conversation-only", 1).await]);

    assert!(
        result
            .cumulative_report
            .gaps
            .contains(&CoverageGap::ProjectRelationshipsUnobserved)
    );
    assert_eq!(
        result.cumulative_report.completeness,
        Completeness::StructurallyPartial
    );
}

#[tokio::test]
async fn per_archive_report_counts_fixture_evidence() {
    let mut parsed = parsed_fixture().await;
    let alpha = parsed
        .conversations
        .iter_mut()
        .find(|conversation| conversation.external_id == "conversation-alpha")
        .expect("alpha conversation must be present");
    alpha
        .messages
        .iter_mut()
        .find(|message| message.external_id == "message-user")
        .expect("user message must be present")
        .parent_external_id = Some("missing-parent".to_owned());
    let result = ArchiveReconciler.reconcile(&[ArchiveSnapshot {
        archive_id: "report-archive".to_owned(),
        sequence: 1,
        parsed,
    }]);

    let report = &result.archive_reports[0];
    assert_eq!(report.archive_id, "report-archive");
    assert_eq!(report.conversations_discovered, 2);
    assert_eq!(report.messages_discovered, 3);
    assert_eq!(report.orphan_messages, 1);
    assert_eq!(report.parse_warning_count, 0);
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.revision_statistics.new_revisions, 5);
    assert_eq!(report.revision_statistics.reused_revisions, 0);
    assert_eq!(
        report.revision_statistics.present_conversation_observations,
        2
    );
    assert_eq!(
        report.revision_statistics.missing_conversation_observations,
        0
    );
}

#[tokio::test]
async fn cumulative_report_sums_revisions_gaps_and_warnings() {
    let first = snapshot("archive-one", 1).await;
    let mut later = parsed_fixture().await;
    later
        .conversations
        .retain(|conversation| conversation.external_id != "conversation-beta");
    let alpha = later
        .conversations
        .iter_mut()
        .find(|conversation| conversation.external_id == "conversation-alpha")
        .expect("alpha conversation must be present");
    alpha.title = Some("changed title".to_owned());
    alpha
        .messages
        .iter_mut()
        .find(|message| message.external_id == "message-assistant")
        .expect("assistant message must be present")
        .model_slug = Some("changed-model".to_owned());
    let second = ArchiveSnapshot {
        archive_id: "archive-two".to_owned(),
        sequence: 2,
        parsed: later,
    };

    let report = ArchiveReconciler
        .reconcile(&[first, second])
        .cumulative_report;

    assert_eq!(report.unique_conversations, 2);
    assert_eq!(report.unique_messages, 3);
    assert_eq!(report.conversation_revisions, 3);
    assert_eq!(report.message_revisions, 4);
    assert_eq!(report.parse_warning_count, 0);
    assert_eq!(report.revision_statistics.new_revisions, 7);
    assert_eq!(report.revision_statistics.reused_revisions, 1);
    assert_eq!(
        report.revision_statistics.present_conversation_observations,
        3
    );
    assert_eq!(
        report.revision_statistics.missing_conversation_observations,
        1
    );
    assert!(
        report
            .gaps
            .contains(&CoverageGap::ProjectRelationshipsUnobserved)
    );
    assert!(report.gaps.contains(&CoverageGap::AssetsUnobserved));
    assert_eq!(report.completeness, Completeness::StructurallyPartial);
}

#[tokio::test]
async fn reconciliation_reports_are_deterministic() {
    let snapshots = vec![
        snapshot("archive-one", 1).await,
        snapshot("archive-two", 2).await,
    ];

    let first = ArchiveReconciler.reconcile(&snapshots);
    let second = ArchiveReconciler.reconcile(&snapshots);

    assert_eq!(first.cumulative_report.revision_statistics.new_revisions, 5);
    assert_eq!(first.archive_reports, second.archive_reports);
    assert_eq!(first.cumulative_report, second.cumulative_report);
}

#[tokio::test]
async fn project_and_asset_revisions_are_append_only() {
    let first = ArchiveSnapshot {
        archive_id: "archive-project-one".to_owned(),
        sequence: 1,
        parsed: parsed_archive_fixture().await,
    };
    let mut later = parsed_archive_fixture().await;
    later.projects[0].title = Some("revised project".to_owned());
    later.assets[0].display_name = Some("revised-note.txt".to_owned());
    let second = ArchiveSnapshot {
        archive_id: "archive-project-two".to_owned(),
        sequence: 2,
        parsed: later,
    };

    let result = ArchiveReconciler.reconcile(&[first, second]);

    assert_eq!(result.projects.len(), 1);
    assert_eq!(result.projects[0].revisions.len(), 2);
    assert_eq!(result.projects[0].instructions.len(), 2);
    assert_eq!(result.assets.len(), 1);
    assert_eq!(result.assets[0].revisions.len(), 2);
    assert_eq!(result.canvas_documents.len(), 1);
}

#[tokio::test]
async fn missing_project_evidence_is_an_observation_not_a_deletion() {
    let first = ArchiveSnapshot {
        archive_id: "archive-project-one".to_owned(),
        sequence: 1,
        parsed: parsed_archive_fixture().await,
    };
    let mut later = parsed_archive_fixture().await;
    later.projects.clear();
    later.canvas_documents.clear();
    later.assets.clear();
    let second = ArchiveSnapshot {
        archive_id: "archive-project-two".to_owned(),
        sequence: 2,
        parsed: later,
    };

    let result = ArchiveReconciler.reconcile(&[first, second]);

    let project = result
        .projects
        .first()
        .expect("project history must remain");
    assert_eq!(project.revisions.len(), 1);
    assert_eq!(project.observations.len(), 2);
    assert_eq!(
        project.observations[1].state,
        ObservationState::MissingFromLatestSnapshot
    );
    assert_eq!(project.observations[1].revision_digest, None);
}

#[tokio::test]
async fn missing_instruction_in_present_project_is_an_observation_not_a_deletion() {
    let first = ArchiveSnapshot {
        archive_id: "archive-project-one".to_owned(),
        sequence: 1,
        parsed: parsed_archive_fixture().await,
    };
    let mut later = parsed_archive_fixture().await;
    later.projects[0].instructions.pop();
    let second = ArchiveSnapshot {
        archive_id: "archive-project-two".to_owned(),
        sequence: 2,
        parsed: later,
    };

    let result = ArchiveReconciler.reconcile(&[first, second]);

    let removed_instruction = result.projects[0]
        .instructions
        .iter()
        .find(|instruction| instruction.external_id == "prompt-alpha")
        .expect("instruction history must remain");
    assert_eq!(removed_instruction.revisions.len(), 1);
    assert_eq!(removed_instruction.observations.len(), 2);
    assert_eq!(
        removed_instruction.observations[1].state,
        ObservationState::MissingFromLatestSnapshot
    );
    assert_eq!(removed_instruction.observations[1].revision_digest, None);
}

#[tokio::test]
async fn quarantined_asset_keeps_completeness_partial() {
    let result = ArchiveReconciler.reconcile(&[ArchiveSnapshot {
        archive_id: "archive-quarantined-asset".to_owned(),
        sequence: 1,
        parsed: parsed_archive_fixture().await,
    }]);

    let report = &result.archive_reports[0];
    assert_eq!(report.projects_discovered, 1);
    assert_eq!(report.instructions_discovered, 2);
    assert_eq!(report.canvas_documents_discovered, 1);
    assert_eq!(report.asset_references_discovered, 1);
    assert_eq!(report.verified_assets, 0);
    assert_eq!(report.missing_assets, 0);
    assert_eq!(report.quarantined_assets, 1);
    assert_eq!(report.completeness, Completeness::StructurallyPartial);
    assert_eq!(result.cumulative_report.unique_projects, 1);
    assert_eq!(result.cumulative_report.unique_assets, 1);
    assert_eq!(
        result.cumulative_report.completeness,
        Completeness::StructurallyPartial
    );
}
