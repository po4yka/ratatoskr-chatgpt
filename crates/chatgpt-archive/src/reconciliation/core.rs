//! Revision-chain reconciliation for present archive evidence.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    ArchiveCompletenessReport, ArchiveSnapshot, Completeness, ConversationHistory, CoverageGap,
    CumulativeCompletenessReport, MessageHistory, Observation, ObservationState,
    ReconciliationResult, ReconciliationWarning, Revision, RevisionStatistics, WarningCode,
};
use crate::ParsedConversation;

pub(super) fn reconcile(snapshots: &[ArchiveSnapshot]) -> ReconciliationResult {
    let mut histories = BTreeMap::new();
    let mut archive_reports = Vec::new();
    for snapshot in snapshots {
        let graph = graph_evidence(snapshot);
        let mut statistics = RevisionStatistics {
            present_conversation_observations: snapshot.parsed.conversations.len(),
            ..RevisionStatistics::default()
        };
        let present_ids = snapshot
            .parsed
            .conversations
            .iter()
            .map(|conversation| conversation.external_id.as_str())
            .collect::<BTreeSet<_>>();
        for conversation in &snapshot.parsed.conversations {
            let history = histories
                .entry(conversation.external_id.clone())
                .or_insert_with(|| ConversationHistory {
                    external_id: conversation.external_id.clone(),
                    revisions: Vec::new(),
                    observations: Vec::new(),
                    messages: Vec::new(),
                });
            statistics = add_statistics(
                statistics,
                observe_conversation(
                    history,
                    conversation,
                    &snapshot.archive_id,
                    &graph.orphaned_messages,
                ),
            );
        }
        for (external_id, history) in &mut histories {
            if !present_ids.contains(external_id.as_str()) {
                history.observations.push(Observation {
                    archive_id: snapshot.archive_id.clone(),
                    state: ObservationState::MissingFromLatestSnapshot,
                    revision_digest: None,
                    orphaned: false,
                });
                statistics.missing_conversation_observations += 1;
            }
        }
        archive_reports.push(archive_report(snapshot, graph.warnings, statistics));
    }
    let conversations = histories.into_values().collect::<Vec<_>>();
    let cumulative_report = cumulative_report(&conversations, &archive_reports);
    ReconciliationResult {
        conversations,
        archive_reports,
        cumulative_report,
    }
}

fn observe_conversation(
    history: &mut ConversationHistory,
    conversation: &ParsedConversation,
    archive_id: &str,
    orphaned_messages: &BTreeSet<(String, String)>,
) -> RevisionStatistics {
    let mut statistics = RevisionStatistics::default();
    let digest = super::digest::conversation_digest(conversation);
    statistics.new_revisions += usize::from(record_present(
        &mut history.revisions,
        &mut history.observations,
        archive_id,
        digest,
        false,
    ));
    let mut messages = history
        .messages
        .drain(..)
        .map(|history| (history.external_id.clone(), history))
        .collect::<BTreeMap<_, _>>();
    for message in &conversation.messages {
        let history = messages
            .entry(message.external_id.clone())
            .or_insert_with(|| MessageHistory {
                external_id: message.external_id.clone(),
                revisions: Vec::new(),
                observations: Vec::new(),
            });
        let digest = super::digest::message_digest(message);
        let created = record_present(
            &mut history.revisions,
            &mut history.observations,
            archive_id,
            digest,
            orphaned_messages.contains(&(
                conversation.external_id.clone(),
                message.external_id.clone(),
            )),
        );
        if created {
            statistics.new_revisions += 1;
        } else {
            statistics.reused_revisions += 1;
        }
    }
    history.messages = messages.into_values().collect();
    statistics
}

fn record_present(
    revisions: &mut Vec<Revision>,
    observations: &mut Vec<Observation>,
    archive_id: &str,
    digest: String,
    orphaned: bool,
) -> bool {
    let created = !revisions.iter().any(|revision| revision.digest == digest);
    if created {
        revisions.push(Revision {
            digest: digest.clone(),
        });
    }
    observations.push(Observation {
        archive_id: archive_id.to_owned(),
        state: ObservationState::Present,
        revision_digest: Some(digest),
        orphaned,
    });
    created
}

fn graph_evidence(snapshot: &ArchiveSnapshot) -> GraphEvidence {
    let all_messages = snapshot
        .parsed
        .conversations
        .iter()
        .flat_map(|conversation| {
            conversation.messages.iter().map(move |message| {
                (
                    message.external_id.as_str(),
                    conversation.external_id.as_str(),
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    let mut evidence = GraphEvidence::default();
    for conversation in &snapshot.parsed.conversations {
        let local_messages = conversation
            .messages
            .iter()
            .map(|message| message.external_id.as_str())
            .collect::<BTreeSet<_>>();
        for message in &conversation.messages {
            let Some(parent) = message.parent_external_id.as_deref() else {
                continue;
            };
            let code = if parent == message.external_id {
                Some(WarningCode::SelfParent)
            } else if local_messages.contains(parent) {
                None
            } else if all_messages.contains_key(parent) {
                Some(WarningCode::CrossConversationParent)
            } else {
                Some(WarningCode::MissingParent)
            };
            if let Some(code) = code {
                evidence.orphaned_messages.insert((
                    conversation.external_id.clone(),
                    message.external_id.clone(),
                ));
                evidence.warnings.push(ReconciliationWarning {
                    code,
                    conversation_external_id: conversation.external_id.clone(),
                    message_external_id: Some(message.external_id.clone()),
                });
            }
        }
    }
    evidence.warnings.sort();
    evidence
}

fn archive_report(
    snapshot: &ArchiveSnapshot,
    warnings: Vec<ReconciliationWarning>,
    revision_statistics: RevisionStatistics,
) -> ArchiveCompletenessReport {
    ArchiveCompletenessReport {
        archive_id: snapshot.archive_id.clone(),
        schema_id: snapshot.parsed.schema_id.clone(),
        parser_id: format!(
            "{}@{}",
            snapshot.parsed.parser.name, snapshot.parsed.parser.version
        ),
        conversations_discovered: snapshot.parsed.conversations.len(),
        messages_discovered: snapshot
            .parsed
            .conversations
            .iter()
            .map(|conversation| conversation.messages.len())
            .sum(),
        orphan_messages: warnings.len(),
        parse_warning_count: 0,
        warnings,
        gaps: vec![
            CoverageGap::ProjectRelationshipsUnobserved,
            CoverageGap::AssetsUnobserved,
        ],
        revision_statistics,
        completeness: Completeness::StructurallyPartial,
    }
}

fn cumulative_report(
    conversations: &[ConversationHistory],
    archive_reports: &[ArchiveCompletenessReport],
) -> CumulativeCompletenessReport {
    let warnings = archive_reports
        .iter()
        .flat_map(|report| report.warnings.clone())
        .collect::<Vec<_>>();
    let gaps = archive_reports
        .iter()
        .flat_map(|report| report.gaps.iter().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let revision_statistics = archive_reports
        .iter()
        .fold(RevisionStatistics::default(), |statistics, report| {
            add_statistics(statistics, report.revision_statistics)
        });
    let parse_warning_count = archive_reports
        .iter()
        .map(|report| report.parse_warning_count)
        .sum();
    CumulativeCompletenessReport {
        unique_conversations: conversations.len(),
        unique_messages: conversations
            .iter()
            .map(|conversation| conversation.messages.len())
            .sum(),
        conversation_revisions: conversations
            .iter()
            .map(|conversation| conversation.revisions.len())
            .sum(),
        message_revisions: conversations
            .iter()
            .flat_map(|conversation| &conversation.messages)
            .map(|message| message.revisions.len())
            .sum(),
        warnings,
        parse_warning_count,
        gaps,
        revision_statistics,
        completeness: Completeness::StructurallyPartial,
    }
}

#[derive(Default)]
struct GraphEvidence {
    warnings: Vec<ReconciliationWarning>,
    orphaned_messages: BTreeSet<(String, String)>,
}

fn add_statistics(
    mut total: RevisionStatistics,
    addition: RevisionStatistics,
) -> RevisionStatistics {
    total.new_revisions += addition.new_revisions;
    total.reused_revisions += addition.reused_revisions;
    total.present_conversation_observations += addition.present_conversation_observations;
    total.missing_conversation_observations += addition.missing_conversation_observations;
    total
}
