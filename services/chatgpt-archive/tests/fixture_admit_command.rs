//! Process boundary for read-only fixture admission.

use std::process::Command;

use ratatoskr_chatgpt_archive_service::{FixtureAdmitCommand, parse_fixture_admit_command};

#[test]
fn fixture_admit_requires_one_candidate_and_reports_refusal_as_json() {
    let parsed = parse_fixture_admit_command(["--candidate", "/private/candidate"]);
    assert_eq!(
        parsed,
        Ok(FixtureAdmitCommand {
            candidate: "/private/candidate".into(),
        })
    );
    assert!(parse_fixture_admit_command([] as [&str; 0]).is_err());
    assert!(parse_fixture_admit_command(["--candidate", "one", "--candidate", "two"]).is_err());
    let missing = tempfile::tempdir()
        .expect("temporary parent")
        .path()
        .join("missing-candidate");
    let output = Command::new(env!("CARGO_BIN_EXE_ratatoskr-chatgpt-archive"))
        .args(["fixture-admit", "--candidate"])
        .arg(&missing)
        .output()
        .expect("binary starts");
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("one JSON report");
    assert_eq!(report["status"], "rejected");
    assert_eq!(
        report["findings"],
        serde_json::json!(["candidate_not_directory"])
    );
}
