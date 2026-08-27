//! Portable-export command contract tests.

use ratatoskr_chatgpt_archive_service::parse_portable_export_command;

#[test]
fn portable_export_command_requires_tenant_and_output() {
    let missing_tenant = parse_portable_export_command(["--output", "archive.zip"]);
    assert!(missing_tenant.is_err(), "tenant scope must be required");

    let missing_output = parse_portable_export_command(["--tenant", "account-alpha"]);
    assert!(missing_output.is_err(), "output path must be required");

    let parsed = parse_portable_export_command([
        "--tenant",
        "account-alpha",
        "--output",
        "archive.zip",
        "--project",
        "project-alpha",
        "--from",
        "2026-08-27T00:00:00Z",
        "--to",
        "2026-08-27T23:59:59Z",
    ])
    .expect("complete portable-export arguments must parse");
    assert_eq!(parsed.filter.account_external_ref, "account-alpha");
    assert_eq!(
        parsed.filter.project_external_id.as_deref(),
        Some("project-alpha")
    );
    assert_eq!(parsed.output.as_os_str(), "archive.zip");
}
