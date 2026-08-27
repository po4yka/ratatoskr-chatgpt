//! Process and parser contracts for tenant parser migrations.

use std::process::Command;

use ratatoskr_chatgpt_archive::ParserId;
use ratatoskr_chatgpt_archive_service::{ParserMigrateCommand, parse_parser_migrate_command};
use uuid::Uuid;

#[test]
fn parser_migrate_command_requires_tenant_parser_and_preserves_dry_run() {
    let tenant = Uuid::now_v7();
    let parsed = parse_parser_migrate_command([
        "--tenant",
        &tenant.to_string(),
        "--parser",
        "chatgpt-export@2.0",
        "--dry-run",
    ]);
    assert_eq!(
        parsed,
        Ok(ParserMigrateCommand {
            tenant_id: tenant,
            parser: ParserId {
                name: "chatgpt-export".to_owned(),
                version: "2.0".to_owned(),
            },
            dry_run: true,
        })
    );
    assert!(
        parse_parser_migrate_command(["--tenant", &tenant.to_string()]).is_err(),
        "exact target parser is required"
    );
    let output = Command::new(env!("CARGO_BIN_EXE_ratatoskr-chatgpt-archive"))
        .args(["parser-migrate", "--tenant", &tenant.to_string()])
        .output()
        .expect("binary starts");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty(), "invalid invocation has no report");
}
