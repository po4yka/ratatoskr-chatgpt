//! Lifecycle operator command execution and process exit mapping.

use std::process::ExitCode;
use std::sync::Arc;

use ratatoskr_chatgpt_archive::fixture_admission::{FixtureAdmission, FixtureAdmissionStatus};
use ratatoskr_chatgpt_archive::parser_migration::{ParserMigrationEngine, ParserMigrationStatus};
use ratatoskr_chatgpt_archive::privacy_deletion::{PrivacyDeletionError, PrivacyDeletionService};
use ratatoskr_chatgpt_archive::reparse::{ReparseEngine, ReparseError};
use ratatoskr_chatgpt_archive::{BlobStore, Database, ParserRegistry};
use uuid::Uuid;

use super::{
    Config, FixtureAdmitCommand, ParserMigrateCommand, PortableExportCommand, PrivacyDeleteCommand,
    ReparseCommand, ServiceError, parse_fixture_admit_command, parse_parser_migrate_command,
    parse_portable_export_command, parse_privacy_delete_command, parse_reparse_command, run,
    run_portable_export,
};

struct PrivacyDeleteOutcome {
    report: serde_json::Value,
    succeeded: bool,
}

async fn run_privacy_delete(
    config: &Config,
    command: &PrivacyDeleteCommand,
) -> Result<PrivacyDeleteOutcome, ServiceError> {
    let root = config
        .storage
        .blob_root
        .clone()
        .ok_or(ServiceError::MissingBlobRoot)?;
    let database = Database::connect(&config.storage, &config.limits).await?;
    database.apply_schema().await?;
    let service = PrivacyDeletionService::new(database.pool().clone(), BlobStore::new(&root)?);
    match command {
        PrivacyDeleteCommand::Plan {
            tenant_id,
            request_id,
            scope,
        } => match service.plan(*tenant_id, *request_id, *scope).await? {
            Some(plan) => Ok(PrivacyDeleteOutcome {
                report: serde_json::to_value(plan).map_err(PrivacyDeletionError::Encode)?,
                succeeded: true,
            }),
            None => Ok(PrivacyDeleteOutcome {
                report: serde_json::json!({
                    "request_id": request_id,
                    "status": "not_found"
                }),
                succeeded: false,
            }),
        },
        PrivacyDeleteCommand::Execute {
            tenant_id,
            request_id,
        } => {
            let report = service.execute_for_tenant(*tenant_id, *request_id).await?;
            Ok(PrivacyDeleteOutcome {
                report: serde_json::to_value(report).map_err(PrivacyDeletionError::Encode)?,
                succeeded: true,
            })
        }
    }
}

async fn run_reparse(
    config: &Config,
    command: &ReparseCommand,
) -> Result<serde_json::Value, ServiceError> {
    let root = config
        .storage
        .blob_root
        .clone()
        .ok_or(ServiceError::MissingBlobRoot)?;
    let database = Database::connect(&config.storage, &config.limits).await?;
    database.apply_schema().await?;
    let engine = ReparseEngine::new(
        database.pool().clone(),
        BlobStore::new(&root)?,
        Arc::new(ParserRegistry::runtime()?),
        (&config.limits).into(),
    );
    let plan = engine
        .plan(
            command.tenant_id,
            command.archive_id,
            command.parser.clone(),
        )
        .await?;
    let report = if command.dry_run {
        plan.report
    } else {
        engine.apply(&plan).await?
    };
    serde_json::to_value(report)
        .map_err(ReparseError::Encode)
        .map_err(Into::into)
}

async fn run_parser_migrate(
    config: &Config,
    command: &ParserMigrateCommand,
) -> Result<ratatoskr_chatgpt_archive::parser_migration::ParserMigrationReport, ServiceError> {
    let root = config
        .storage
        .blob_root
        .clone()
        .ok_or(ServiceError::MissingBlobRoot)?;
    let database = Database::connect(&config.storage, &config.limits).await?;
    database.apply_schema().await?;
    let reparse = ReparseEngine::new(
        database.pool().clone(),
        BlobStore::new(&root)?,
        Arc::new(ParserRegistry::runtime()?),
        (&config.limits).into(),
    );
    let engine = ParserMigrationEngine::new(reparse);
    let plan = engine
        .plan(Uuid::now_v7(), command.tenant_id, command.parser.clone())
        .await?;
    if command.dry_run {
        Ok(plan.report)
    } else {
        engine.apply(&plan).await.map_err(Into::into)
    }
}

/// The process entry point: configuration failures exit 78 (`EX_CONFIG`), other
/// startup failures exit 1 with a value-free stderr line.
///
/// # Panics
///
/// Never reaches a request path: a process whose async runtime cannot be built
/// cannot even reach configuration loading.
#[must_use]
#[allow(
    clippy::expect_used,
    reason = "runtime construction can only fail through an invalid builder; there is no \
              meaningful refusal path for a process that cannot schedule at all"
)]
pub fn main_result() -> ExitCode {
    let mut arguments = std::env::args();
    let _program = arguments.next();
    let subcommand = arguments.next();
    if subcommand.as_deref() == Some("portable-export") {
        let Ok(command) = parse_portable_export_command(arguments) else {
            eprintln!("ratatoskr-chatgpt-archive: portable-export requires --tenant and --output");
            return ExitCode::from(64);
        };
        return portable_export_main(&command);
    }
    if subcommand.as_deref() == Some("privacy-delete") {
        let Ok(command) = parse_privacy_delete_command(arguments) else {
            eprintln!("ratatoskr-chatgpt-archive: privacy-delete arguments are invalid");
            return ExitCode::from(2);
        };
        return privacy_delete_main(&command);
    }
    if subcommand.as_deref() == Some("reparse") {
        let Ok(command) = parse_reparse_command(arguments) else {
            eprintln!("ratatoskr-chatgpt-archive: reparse arguments are invalid");
            return ExitCode::from(2);
        };
        return reparse_main(&command);
    }
    if subcommand.as_deref() == Some("parser-migrate") {
        let Ok(command) = parse_parser_migrate_command(arguments) else {
            eprintln!("ratatoskr-chatgpt-archive: parser-migrate arguments are invalid");
            return ExitCode::from(2);
        };
        return parser_migrate_main(&command);
    }
    if subcommand.as_deref() == Some("fixture-admit") {
        let Ok(command) = parse_fixture_admit_command(arguments) else {
            eprintln!("ratatoskr-chatgpt-archive: fixture-admit arguments are invalid");
            return ExitCode::from(2);
        };
        return fixture_admit_main(&command);
    }
    let config = match Config::load() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("ratatoskr-chatgpt-archive: {error}");
            return ExitCode::from(78);
        }
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("building the async runtime must work");
    let outcome = runtime.block_on(run(&config));
    runtime.shutdown_timeout(std::time::Duration::from_millis(
        config.limits.shutdown_timeout_ms,
    ));

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(chain = %error, "the service stopped");
            eprintln!("ratatoskr-chatgpt-archive: the service stopped");
            ExitCode::FAILURE
        }
    }
}

#[allow(
    clippy::expect_used,
    reason = "runtime construction can only fail through an invalid builder"
)]
fn portable_export_main(command: &PortableExportCommand) -> ExitCode {
    let config = match Config::load() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("ratatoskr-chatgpt-archive: {error}");
            return ExitCode::from(78);
        }
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("building the async runtime must work");
    let outcome = runtime.block_on(run_portable_export(&config, command));
    runtime.shutdown_timeout(std::time::Duration::from_millis(
        config.limits.shutdown_timeout_ms,
    ));
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(chain = %error, "portable export failed");
            eprintln!("ratatoskr-chatgpt-archive: portable export failed");
            ExitCode::FAILURE
        }
    }
}

#[allow(
    clippy::expect_used,
    reason = "runtime construction can only fail through an invalid builder"
)]
fn privacy_delete_main(command: &PrivacyDeleteCommand) -> ExitCode {
    let config = match Config::load() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("ratatoskr-chatgpt-archive: {error}");
            return ExitCode::from(2);
        }
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("building the async runtime must work");
    let outcome = runtime.block_on(run_privacy_delete(&config, command));
    runtime.shutdown_timeout(std::time::Duration::from_millis(
        config.limits.shutdown_timeout_ms,
    ));
    match outcome {
        Ok(outcome) => {
            if !write_json_stdout(&outcome.report) {
                return ExitCode::FAILURE;
            }
            if outcome.succeeded {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            let request_id = match command {
                PrivacyDeleteCommand::Plan { request_id, .. }
                | PrivacyDeleteCommand::Execute { request_id, .. } => request_id,
            };
            let report = serde_json::json!({
                "request_id": request_id,
                "status": "failed",
                "error_code": "operation_failed"
            });
            let _ = write_json_stdout(&report);
            tracing::error!(chain = %error, "privacy deletion failed");
            eprintln!("ratatoskr-chatgpt-archive: privacy deletion failed");
            ExitCode::FAILURE
        }
    }
}

#[allow(
    clippy::expect_used,
    reason = "runtime construction can only fail through an invalid builder"
)]
fn reparse_main(command: &ReparseCommand) -> ExitCode {
    let config = match Config::load() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("ratatoskr-chatgpt-archive: {error}");
            return ExitCode::from(2);
        }
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("building the async runtime must work");
    let outcome = runtime.block_on(run_reparse(&config, command));
    runtime.shutdown_timeout(std::time::Duration::from_millis(
        config.limits.shutdown_timeout_ms,
    ));
    match outcome {
        Ok(report) if write_json_stdout(&report) => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(error) => {
            let report = serde_json::json!({
                "archive_id": command.archive_id,
                "parser": command.parser,
                "status": "failed",
                "error_code": "operation_failed"
            });
            let _ = write_json_stdout(&report);
            tracing::error!(chain = %error, "reparse failed");
            eprintln!("ratatoskr-chatgpt-archive: reparse failed");
            ExitCode::FAILURE
        }
    }
}

#[allow(
    clippy::expect_used,
    reason = "runtime construction can only fail through an invalid builder"
)]
fn parser_migrate_main(command: &ParserMigrateCommand) -> ExitCode {
    let config = match Config::load() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("ratatoskr-chatgpt-archive: {error}");
            return ExitCode::from(2);
        }
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("building the async runtime must work");
    let outcome = runtime.block_on(run_parser_migrate(&config, command));
    runtime.shutdown_timeout(std::time::Duration::from_millis(
        config.limits.shutdown_timeout_ms,
    ));
    match outcome {
        Ok(report) => {
            let partial = report.status == ParserMigrationStatus::Partial;
            let Ok(value) = serde_json::to_value(report) else {
                return ExitCode::FAILURE;
            };
            if !write_json_stdout(&value) || partial {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => {
            let report = serde_json::json!({
                "tenant_id": command.tenant_id,
                "parser": command.parser,
                "status": "failed",
                "error_code": "operation_failed"
            });
            let _ = write_json_stdout(&report);
            tracing::error!(chain = %error, "parser migration failed");
            eprintln!("ratatoskr-chatgpt-archive: parser migration failed");
            ExitCode::FAILURE
        }
    }
}

fn fixture_admit_main(command: &FixtureAdmitCommand) -> ExitCode {
    if let Ok(report) = FixtureAdmission::inspect(&command.candidate) {
        let admitted = report.status == FixtureAdmissionStatus::Admitted;
        let Ok(value) = serde_json::to_value(report) else {
            return ExitCode::FAILURE;
        };
        if !write_json_stdout(&value) || !admitted {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        }
    } else {
        let report = serde_json::json!({
            "case_id": null,
            "status": "rejected",
            "findings": ["candidate_unreadable"]
        });
        let _ = write_json_stdout(&report);
        eprintln!("ratatoskr-chatgpt-archive: fixture admission failed");
        ExitCode::FAILURE
    }
}

fn write_json_stdout(value: &serde_json::Value) -> bool {
    use std::io::Write as _;

    let mut encoded = value.to_string().into_bytes();
    encoded.push(b'\n');
    match std::io::stdout().lock().write_all(&encoded) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => true,
        Err(_) => {
            eprintln!("ratatoskr-chatgpt-archive: writing command report failed");
            false
        }
    }
}
