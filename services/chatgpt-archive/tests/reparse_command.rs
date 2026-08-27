//! Process and parser contracts for explicit reparse commands.

use std::process::Command;

use ratatoskr_chatgpt_archive::ParserId;
use ratatoskr_chatgpt_archive_service::{ReparseCommand, parse_reparse_command};
use uuid::Uuid;

#[test]
fn reparse_command_requires_tenant_archive_parser_and_preserves_dry_run() {
    let tenant = Uuid::now_v7();
    let archive = Uuid::now_v7();
    let parsed = parse_reparse_command([
        "--tenant",
        &tenant.to_string(),
        "--archive",
        &archive.to_string(),
        "--parser",
        "chatgpt-export@2.0",
        "--dry-run",
    ]);
    assert_eq!(
        parsed,
        Ok(ReparseCommand {
            tenant_id: tenant,
            archive_id: archive,
            parser: ParserId {
                name: "chatgpt-export".to_owned(),
                version: "2.0".to_owned(),
            },
            dry_run: true,
        })
    );
    assert!(
        parse_reparse_command([
            "--tenant",
            &tenant.to_string(),
            "--archive",
            &archive.to_string(),
        ])
        .is_err(),
        "an exact parser is required"
    );
    assert!(
        parse_reparse_command([
            "--tenant",
            &tenant.to_string(),
            "--archive",
            &archive.to_string(),
            "--parser",
            "chatgpt-export",
        ])
        .is_err(),
        "parser identity must be exact NAME@VERSION"
    );
    let output = Command::new(env!("CARGO_BIN_EXE_ratatoskr-chatgpt-archive"))
        .args([
            "reparse",
            "--tenant",
            &tenant.to_string(),
            "--archive",
            &archive.to_string(),
        ])
        .output()
        .expect("binary starts");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty(), "invalid invocation has no report");
}
