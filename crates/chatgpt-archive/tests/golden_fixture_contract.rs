//! Read-only contract for admitted owner-derived fixture manifests.

use std::path::PathBuf;

use ratatoskr_chatgpt_archive::fixture_admission::{FixtureAdmission, FixtureAdmissionStatus};

#[test]
fn admitted_golden_is_synthetic_deterministic_and_read_only()
-> Result<(), Box<dyn std::error::Error>> {
    let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/owner-derived/synthetic-conversation-shape");
    let before = snapshot(&candidate)?;
    let report = FixtureAdmission::inspect(&candidate)?;
    let after = snapshot(&candidate)?;
    assert_eq!(
        report.status,
        FixtureAdmissionStatus::Admitted,
        "{report:?}"
    );
    assert_eq!(
        report.case_id.as_deref(),
        Some("synthetic-conversation-shape")
    );
    assert_eq!(
        before, after,
        "ordinary tests must never bless or rewrite goldens"
    );
    let expected: serde_json::Value =
        serde_json::from_slice(&std::fs::read(candidate.join("expected-structure.json"))?)?;
    assert_eq!(expected["parser_selection"], "unsupported");
    assert_eq!(expected["completeness"], "unknown");
    Ok(())
}

fn snapshot(root: &std::path::Path) -> std::io::Result<Vec<(String, Vec<u8>)>> {
    let mut files = std::fs::read_dir(root)?
        .map(|entry| {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            Ok((name, std::fs::read(entry.path())?))
        })
        .collect::<std::io::Result<Vec<_>>>()?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}
