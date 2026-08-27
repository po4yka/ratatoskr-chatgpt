//! Process and parser contracts for explicit privacy deletion commands.

use std::process::Command;

use ratatoskr_chatgpt_archive::privacy_deletion::PrivacyDeletionScope;
use ratatoskr_chatgpt_archive_service::{PrivacyDeleteCommand, parse_privacy_delete_command};
use uuid::Uuid;

#[test]
fn privacy_delete_plan_requires_exactly_one_tenant_scope() {
    let tenant = Uuid::now_v7();
    let request = Uuid::now_v7();
    let archive = Uuid::now_v7();
    let valid = parse_privacy_delete_command([
        "plan",
        "--tenant",
        &tenant.to_string(),
        "--request",
        &request.to_string(),
        "--archive",
        &archive.to_string(),
    ]);
    assert!(
        matches!(
            valid,
            Ok(PrivacyDeleteCommand::Plan {
                scope: PrivacyDeletionScope::Archive { .. },
                ..
            })
        ),
        "one complete archive scope must parse: {valid:?}"
    );
    assert!(
        parse_privacy_delete_command([
            "plan",
            "--tenant",
            &tenant.to_string(),
            "--request",
            &request.to_string(),
        ])
        .is_err(),
        "a plan without scope must be rejected"
    );
    let output = Command::new(env!("CARGO_BIN_EXE_ratatoskr-chatgpt-archive"))
        .args([
            "privacy-delete",
            "plan",
            "--tenant",
            &tenant.to_string(),
            "--request",
            &request.to_string(),
            "--archive",
            &archive.to_string(),
            "--all",
        ])
        .output()
        .expect("binary starts");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty(), "invalid invocation has no report");
}

#[test]
fn privacy_delete_execute_requires_confirmation() {
    let tenant = Uuid::now_v7();
    let request = Uuid::now_v7();
    let missing_confirmation = parse_privacy_delete_command([
        "execute",
        "--tenant",
        &tenant.to_string(),
        "--request",
        &request.to_string(),
    ]);
    assert!(
        missing_confirmation.is_err(),
        "destructive execution must require --confirm"
    );
    let confirmed = parse_privacy_delete_command([
        "execute",
        "--tenant",
        &tenant.to_string(),
        "--request",
        &request.to_string(),
        "--confirm",
    ]);
    assert!(
        matches!(confirmed, Ok(PrivacyDeleteCommand::Execute { .. })),
        "confirmed execution must parse: {confirmed:?}"
    );
    let output = Command::new(env!("CARGO_BIN_EXE_ratatoskr-chatgpt-archive"))
        .args([
            "privacy-delete",
            "execute",
            "--tenant",
            &tenant.to_string(),
            "--request",
            &request.to_string(),
        ])
        .output()
        .expect("binary starts");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty(), "invalid invocation has no report");
}
