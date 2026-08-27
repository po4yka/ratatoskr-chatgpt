//! Revision-chain reconciliation for present archive evidence.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    ArchiveCompletenessReport, ArchiveSnapshot, AssetHistory, CanvasDocumentHistory, Completeness,
    ConversationHistory, CoverageGap, CumulativeCompletenessReport, InstructionHistory,
    MessageHistory, Observation, ObservationState, ProjectHistory, ReconciliationResult,
    ReconciliationWarning, Revision, RevisionStatistics, WarningCode,
};
use crate::{
    AssetAvailability, ParsedAsset, ParsedCanvasDocument, ParsedConversation, ParsedInstruction,
    ParsedProject,
};

pub(super) fn reconcile(snapshots: &[ArchiveSnapshot]) -> ReconciliationResult {
    let mut histories = BTreeMap::new();
    let mut projects = BTreeMap::new();
    let mut canvas_documents = BTreeMap::new();
    let mut assets = BTreeMap::new();
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
        reconcile_additional_evidence(snapshot, &mut projects, &mut canvas_documents, &mut assets);
        archive_reports.push(archive_report(snapshot, graph.warnings, statistics));
    }
    let conversations = histories.into_values().collect::<Vec<_>>();
    let projects = projects.into_values().collect::<Vec<_>>();
    let canvas_documents = canvas_documents.into_values().collect::<Vec<_>>();
    let assets = assets.into_values().collect::<Vec<_>>();
    let cumulative_report = cumulative_report(
        &conversations,
        &projects,
        &canvas_documents,
        &assets,
        &archive_reports,
    );
    ReconciliationResult {
        conversations,
        projects,
        canvas_documents,
        assets,
        archive_reports,
        cumulative_report,
    }
}

fn reconcile_additional_evidence(
    snapshot: &ArchiveSnapshot,
    projects: &mut BTreeMap<String, ProjectHistory>,
    canvas_documents: &mut BTreeMap<String, CanvasDocumentHistory>,
    assets: &mut BTreeMap<String, AssetHistory>,
) {
    let present_project_ids = snapshot
        .parsed
        .projects
        .iter()
        .map(|project| project.external_id.as_str())
        .collect::<BTreeSet<_>>();
    observe_projects(projects, &snapshot.parsed.projects, &snapshot.archive_id);
    record_missing_projects(projects, &present_project_ids, &snapshot.archive_id);

    let present_canvas_ids = snapshot
        .parsed
        .canvas_documents
        .iter()
        .map(|document| document.external_id.as_str())
        .collect::<BTreeSet<_>>();
    observe_canvas(
        canvas_documents,
        &snapshot.parsed.canvas_documents,
        &snapshot.archive_id,
    );
    record_missing_canvas(canvas_documents, &present_canvas_ids, &snapshot.archive_id);

    let present_asset_ids = snapshot
        .parsed
        .assets
        .iter()
        .map(|asset| asset.external_id.as_str())
        .collect::<BTreeSet<_>>();
    observe_assets(assets, &snapshot.parsed.assets, &snapshot.archive_id);
    record_missing_assets(assets, &present_asset_ids, &snapshot.archive_id);
}

fn missing_observation(archive_id: &str) -> Observation {
    Observation {
        archive_id: archive_id.to_owned(),
        state: ObservationState::MissingFromLatestSnapshot,
        revision_digest: None,
        orphaned: false,
    }
}

fn record_missing_projects(
    histories: &mut BTreeMap<String, ProjectHistory>,
    present_ids: &BTreeSet<&str>,
    archive_id: &str,
) {
    for (external_id, history) in histories {
        if !present_ids.contains(external_id.as_str()) {
            history.observations.push(missing_observation(archive_id));
            for instruction in &mut history.instructions {
                instruction
                    .observations
                    .push(missing_observation(archive_id));
            }
        }
    }
}

fn record_missing_canvas(
    histories: &mut BTreeMap<String, CanvasDocumentHistory>,
    present_ids: &BTreeSet<&str>,
    archive_id: &str,
) {
    for (external_id, history) in histories {
        if !present_ids.contains(external_id.as_str()) {
            history.observations.push(missing_observation(archive_id));
        }
    }
}

fn record_missing_assets(
    histories: &mut BTreeMap<String, AssetHistory>,
    present_ids: &BTreeSet<&str>,
    archive_id: &str,
) {
    for (external_id, history) in histories {
        if !present_ids.contains(external_id.as_str()) {
            history.observations.push(missing_observation(archive_id));
        }
    }
}

fn observe_projects(
    histories: &mut BTreeMap<String, ProjectHistory>,
    projects: &[ParsedProject],
    archive_id: &str,
) {
    for project in projects {
        let history = histories
            .entry(project.external_id.clone())
            .or_insert_with(|| ProjectHistory {
                external_id: project.external_id.clone(),
                revisions: Vec::new(),
                observations: Vec::new(),
                instructions: Vec::new(),
            });
        record_present(
            &mut history.revisions,
            &mut history.observations,
            archive_id,
            super::digest::project_digest(project),
            false,
        );
        let mut instructions = history
            .instructions
            .drain(..)
            .map(|history| (history.external_id.clone(), history))
            .collect::<BTreeMap<_, _>>();
        let present_instruction_ids = project
            .instructions
            .iter()
            .map(|instruction| instruction_external_id(project, instruction))
            .collect::<BTreeSet<_>>();
        for instruction in &project.instructions {
            let external_id = instruction_external_id(project, instruction);
            let instruction_history =
                instructions
                    .entry(external_id.clone())
                    .or_insert_with(|| InstructionHistory {
                        external_id,
                        revisions: Vec::new(),
                        observations: Vec::new(),
                    });
            record_present(
                &mut instruction_history.revisions,
                &mut instruction_history.observations,
                archive_id,
                super::digest::instruction_digest(instruction),
                false,
            );
        }
        for (external_id, instruction_history) in &mut instructions {
            if !present_instruction_ids.contains(external_id) {
                instruction_history
                    .observations
                    .push(missing_observation(archive_id));
            }
        }
        history.instructions = instructions.into_values().collect();
    }
}

fn instruction_external_id(project: &ParsedProject, instruction: &ParsedInstruction) -> String {
    instruction
        .external_id
        .clone()
        .unwrap_or_else(|| format!("{}:{}", project.external_id, instruction.ordinal))
}

fn observe_canvas(
    histories: &mut BTreeMap<String, CanvasDocumentHistory>,
    documents: &[ParsedCanvasDocument],
    archive_id: &str,
) {
    for document in documents {
        let history = histories
            .entry(document.external_id.clone())
            .or_insert_with(|| CanvasDocumentHistory {
                external_id: document.external_id.clone(),
                revisions: Vec::new(),
                observations: Vec::new(),
            });
        record_present(
            &mut history.revisions,
            &mut history.observations,
            archive_id,
            super::digest::canvas_digest(document),
            false,
        );
    }
}

fn observe_assets(
    histories: &mut BTreeMap<String, AssetHistory>,
    assets: &[ParsedAsset],
    archive_id: &str,
) {
    for asset in assets {
        let history = histories
            .entry(asset.external_id.clone())
            .or_insert_with(|| AssetHistory {
                external_id: asset.external_id.clone(),
                revisions: Vec::new(),
                observations: Vec::new(),
            });
        record_present(
            &mut history.revisions,
            &mut history.observations,
            archive_id,
            super::digest::asset_digest(asset),
            false,
        );
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
    let verified_assets = snapshot
        .parsed
        .assets
        .iter()
        .filter(|asset| asset.availability == AssetAvailability::Verified)
        .count();
    let missing_assets = snapshot
        .parsed
        .assets
        .iter()
        .filter(|asset| asset.availability == AssetAvailability::Missing)
        .count();
    let quarantined_assets = snapshot
        .parsed
        .assets
        .iter()
        .filter(|asset| asset.availability == AssetAvailability::Quarantined)
        .count();
    let mut gaps = Vec::new();
    if snapshot.parsed.projects.is_empty() {
        gaps.push(CoverageGap::ProjectRelationshipsUnobserved);
    }
    if snapshot.parsed.canvas_documents.is_empty() {
        gaps.push(CoverageGap::CanvasDocumentsUnobserved);
    }
    if snapshot.parsed.assets.is_empty() {
        gaps.push(CoverageGap::AssetsUnobserved);
    }
    if missing_assets > 0 {
        gaps.push(CoverageGap::AssetsMissing);
    }
    if quarantined_assets > 0 {
        gaps.push(CoverageGap::AssetsQuarantined);
    }
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
        projects_discovered: snapshot.parsed.projects.len(),
        instructions_discovered: snapshot
            .parsed
            .projects
            .iter()
            .map(|project| project.instructions.len())
            .sum(),
        canvas_documents_discovered: snapshot.parsed.canvas_documents.len(),
        asset_references_discovered: snapshot.parsed.assets.len(),
        verified_assets,
        missing_assets,
        quarantined_assets,
        orphan_messages: warnings.len(),
        parse_warning_count: 0,
        warnings,
        gaps,
        revision_statistics,
        completeness: Completeness::StructurallyPartial,
    }
}

fn cumulative_report(
    conversations: &[ConversationHistory],
    projects: &[ProjectHistory],
    canvas_documents: &[CanvasDocumentHistory],
    assets: &[AssetHistory],
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
        unique_projects: projects.len(),
        unique_canvas_documents: canvas_documents.len(),
        unique_assets: assets.len(),
        conversation_revisions: conversations
            .iter()
            .map(|conversation| conversation.revisions.len())
            .sum(),
        project_revisions: projects.iter().map(|history| history.revisions.len()).sum(),
        canvas_document_revisions: canvas_documents
            .iter()
            .map(|history| history.revisions.len())
            .sum(),
        asset_revisions: assets.iter().map(|history| history.revisions.len()).sum(),
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
