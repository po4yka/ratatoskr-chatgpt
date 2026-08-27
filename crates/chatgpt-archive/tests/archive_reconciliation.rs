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

use ratatoskr_chatgpt_archive::{
    AcquisitionMode, ArchiveInventory, ArchiveReconciler, ArchiveSnapshot, Completeness,
    CoverageGap, ObservationState, ParserRegistry, ParserSelection, SyntheticConversationsParser,
    WarningCode,
};

const FIXTURE: &[u8] = include_bytes!("fixtures/synthetic_conversations.json");

fn parsed_fixture() -> ratatoskr_chatgpt_archive::ParsedConversations {
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
    SyntheticConversationsParser
        .parse(FIXTURE, &parser)
        .expect("synthetic fixture must parse")
}

fn snapshot(archive_id: &str, sequence: u64) -> ArchiveSnapshot {
    ArchiveSnapshot {
        archive_id: archive_id.to_owned(),
        sequence,
        parsed: parsed_fixture(),
    }
}

#[test]
fn revision_chain_builds_across_fixture_exports() {
    let first = snapshot("archive-one", 1);
    let mut changed = parsed_fixture();
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

#[test]
fn missing_conversation_becomes_observation_not_deletion() {
    let first = snapshot("archive-one", 1);
    let mut later = parsed_fixture();
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

#[test]
fn orphan_parent_is_retained_and_reported() {
    let mut parsed = parsed_fixture();
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

#[test]
fn conversation_only_snapshot_reports_project_relationship_gap() {
    let result = ArchiveReconciler.reconcile(&[snapshot("conversation-only", 1)]);

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

#[test]
fn per_archive_report_counts_fixture_evidence() {
    let mut parsed = parsed_fixture();
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

#[test]
fn cumulative_report_sums_revisions_gaps_and_warnings() {
    let first = snapshot("archive-one", 1);
    let mut later = parsed_fixture();
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

#[test]
fn reconciliation_reports_are_deterministic() {
    let snapshots = vec![snapshot("archive-one", 1), snapshot("archive-two", 2)];

    let first = ArchiveReconciler.reconcile(&snapshots);
    let second = ArchiveReconciler.reconcile(&snapshots);

    assert_eq!(first.cumulative_report.revision_statistics.new_revisions, 5);
    assert_eq!(first.archive_reports, second.archive_reports);
    assert_eq!(first.cumulative_report, second.cumulative_report);
}
