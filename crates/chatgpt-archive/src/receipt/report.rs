//! Typed, bounded terminal reports for Platform-owned archive operations.

use ratatoskr_ai_archive_contracts::platform_receipt::incomplete_import_warning;
use ratatoskr_ai_archive_contracts::{
    AiArchiveCompleteness, AiArchiveOperationSummary, AiProvider,
};
use ratatoskr_error_contracts::{ErrorCode, ErrorEnvelope};
use ratatoskr_identifiers::SafeMessage;
use ratatoskr_identifiers::{AiArchiveId, EntityRef, Extensions, OperationId};
use ratatoskr_operation_contracts::{
    OperationReported, OperationResultKind, OperationResultRef, OperationStatus,
};

use super::repository::{PlatformOperation, RepositoryError};

/// Evidence-based counts produced by one completed import.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ImportSummary {
    pub(crate) completeness: AiArchiveCompleteness,
    pub(crate) conversation_count: u32,
    pub(crate) message_count: u32,
    pub(crate) asset_count: u32,
    pub(crate) gap_count: u32,
    pub(crate) warning_count: u32,
}

/// Produces the terminal Platform result after normalized import truth exists.
pub(crate) fn imported(
    operation: PlatformOperation,
    ai_archive_id: uuid::Uuid,
    summary: ImportSummary,
) -> Result<OperationReported, RepositoryError> {
    let archive = AiArchiveId::parse(&ai_archive_id.to_string()).map_err(invalid_contract)?;
    let operation_id =
        OperationId::parse(&operation.operation_id.to_string()).map_err(invalid_contract)?;
    let provider = AiProvider::parse("chatgpt").map_err(invalid_contract)?;
    let result_kind = OperationResultKind::parse("ai_archive.import").map_err(invalid_contract)?;
    let status = if summary.completeness == AiArchiveCompleteness::Complete {
        OperationStatus::Succeeded
    } else {
        OperationStatus::PartiallySucceeded
    };
    let report = OperationReported {
        operation_id,
        status,
        stage: None,
        progress_percent: None,
        results: vec![OperationResultRef {
            result_kind,
            target: EntityRef::from(archive),
            blob: None,
            ai_archive_import_summary: Some(AiArchiveOperationSummary {
                ai_archive_id: archive,
                provider,
                completeness: summary.completeness,
                conversation_count: summary.conversation_count,
                message_count: summary.message_count,
                asset_count: summary.asset_count,
                gap_count: summary.gap_count,
                warning_count: summary.warning_count,
            }),
            extensions: Extensions::new(),
        }],
        error: None,
        // A partial success without a diagnostic is invalid (S06 D3): an incomplete
        // import carries exactly the contract warning and no error.
        warnings: if status == OperationStatus::PartiallySucceeded {
            vec![incomplete_import_warning()]
        } else {
            Vec::new()
        },
        extensions: Extensions::new(),
    };
    Ok(report)
}

/// Produces the bounded terminal result for provider bytes that can never be imported.
pub(crate) fn failed(operation: PlatformOperation) -> Result<OperationReported, RepositoryError> {
    let operation_id =
        OperationId::parse(&operation.operation_id.to_string()).map_err(invalid_contract)?;
    let mut error = ErrorEnvelope::new(
        ErrorCode::parse("chatgpt.archive.invalid").map_err(invalid_contract)?,
        SafeMessage::parse("The ChatGPT archive could not be imported.")
            .map_err(invalid_contract)?,
        false,
    );
    error.correlation_id = Some(EntityRef::from(operation_id));
    Ok(OperationReported {
        operation_id,
        status: OperationStatus::Failed,
        stage: None,
        progress_percent: None,
        results: Vec::new(),
        error: Some(error),
        warnings: Vec::new(),
        extensions: Extensions::new(),
    })
}

fn invalid_contract(error: impl std::error::Error + Send + Sync + 'static) -> RepositoryError {
    RepositoryError::backend(error)
}

#[cfg(test)]
mod tests {
    use ratatoskr_ai_archive_contracts::AiArchiveCompleteness;
    use ratatoskr_ai_archive_contracts::platform_receipt::incomplete_import_warning;
    use ratatoskr_operation_contracts::OperationStatus;
    use uuid::Uuid;

    use super::{ImportSummary, PlatformOperation, imported};

    fn summary(completeness: AiArchiveCompleteness) -> ImportSummary {
        ImportSummary {
            completeness,
            conversation_count: 1,
            message_count: 2,
            asset_count: 0,
            gap_count: u32::from(completeness != AiArchiveCompleteness::Complete),
            warning_count: 0,
        }
    }

    fn report(
        completeness: AiArchiveCompleteness,
    ) -> ratatoskr_operation_contracts::OperationReported {
        imported(
            PlatformOperation {
                operation_id: Uuid::now_v7(),
            },
            Uuid::now_v7(),
            summary(completeness),
        )
        .expect("the report builds from valid identities")
    }

    #[test]
    fn an_incomplete_import_reports_partial_success_with_the_contract_warning() {
        let report = report(AiArchiveCompleteness::StructurallyPartial);

        assert_eq!(report.status, OperationStatus::PartiallySucceeded);
        assert_eq!(report.warnings, vec![incomplete_import_warning()]);
        assert!(report.error.is_none());
        report
            .validate()
            .expect("an incomplete import must satisfy the report invariant");
    }

    #[test]
    fn a_complete_import_reports_success_without_warnings() {
        let report = report(AiArchiveCompleteness::Complete);

        assert_eq!(report.status, OperationStatus::Succeeded);
        assert!(report.warnings.is_empty());
        report.validate().expect("a complete import is valid");
    }
}
