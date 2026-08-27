//! Typed, bounded terminal reports for Platform-owned archive operations.

use ratatoskr_ai_archive_contracts::{
    AiArchiveCompleteness, AiArchiveOperationSummary, AiProvider,
};
use ratatoskr_identifiers::{AiArchiveId, EntityRef, Extensions, OperationId};
use ratatoskr_operation_contracts::{
    OperationReported, OperationResultKind, OperationResultRef, OperationStatus,
};

use super::repository::{PlatformOperation, RepositoryError};

/// Produces the one truthful terminal fact available after raw evidence is
/// stored: parsing has not yet established normalized coverage.
pub(crate) fn raw_stored_partial(
    operation: PlatformOperation,
    ai_archive_id: uuid::Uuid,
) -> Result<serde_json::Value, RepositoryError> {
    let archive = AiArchiveId::parse(&ai_archive_id.to_string()).map_err(invalid_contract)?;
    let operation_id =
        OperationId::parse(&operation.operation_id.to_string()).map_err(invalid_contract)?;
    let provider = AiProvider::parse("chatgpt").map_err(invalid_contract)?;
    let result_kind = OperationResultKind::parse("ai_archive.import").map_err(invalid_contract)?;
    let report = OperationReported {
        operation_id,
        status: OperationStatus::PartiallySucceeded,
        stage: None,
        progress_percent: None,
        results: vec![OperationResultRef {
            result_kind,
            target: EntityRef::from(archive),
            blob: None,
            ai_archive_import_summary: Some(AiArchiveOperationSummary {
                ai_archive_id: archive,
                provider,
                completeness: AiArchiveCompleteness::Unknown,
                conversation_count: 0,
                message_count: 0,
                asset_count: 0,
                gap_count: 1,
                warning_count: 1,
            }),
            extensions: Extensions::new(),
        }],
        error: None,
        warnings: Vec::new(),
        extensions: Extensions::new(),
    };
    serde_json::to_value(report).map_err(RepositoryError::backend)
}

fn invalid_contract(error: impl std::error::Error + Send + Sync + 'static) -> RepositoryError {
    RepositoryError::backend(error)
}
