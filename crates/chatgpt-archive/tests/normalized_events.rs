//! Published normalized-event conformance and linkage tests.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "synthetic contract fixtures"
)]

use ratatoskr_ai_archive_contracts::{
    AiArchiveImport, AiArchiveProvenance, AiArchiveTombstone, AiConversation, AiConversationAdded,
    AiProject, AiProjectAdded,
};
use ratatoskr_chatgpt_archive::NormalizedArchiveEvent;
use ratatoskr_identifiers::{ContentDigest, DigestAlgorithm, DigestHex, Extensions};

const PROVENANCE: &str = r#"{
  "ai_archive_id":"018f0000-0000-7000-8000-000000000402",
  "provider":"chatgpt", "owner":"user:018f0000-0000-7000-8000-000000000005",
  "source_export":{"owner_service":"ratatoskr-chatgpt","digest":{"algorithm":"sha256","hex":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"media_type":"application/json","length_bytes":512},
  "imported_at":"2026-08-17T10:00:00Z", "parser_name":"chatgpt_export", "parser_version":"2026.08.1"
}"#;

const IMPORT: &str = r#"{
  "ai_archive_id":"018f0000-0000-7000-8000-000000000402",
  "provider":"chatgpt", "owner":"user:018f0000-0000-7000-8000-000000000005",
  "source_export":{"owner_service":"ratatoskr-chatgpt","digest":{"algorithm":"sha256","hex":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"media_type":"application/json","length_bytes":512},
  "imported_at":"2026-08-17T10:00:00Z", "parser_name":"chatgpt_export", "parser_version":"2026.08.1",
  "completeness_report":{"completeness":"complete","conversation_count":1,"message_count":1,"asset_count":0,"gap_count":0}
}"#;

const CONVERSATION: &str = r#"{
  "ai_conversation_id":"018f0000-0000-7000-8000-000000000403",
  "provider":"chatgpt", "owner":"user:018f0000-0000-7000-8000-000000000005",
  "messages":[{"external_message_id":"msg-0001","author_role":"user","parts":[{"part_kind":"text","text":"Evidence."}],"parser_name":"chatgpt_export","parser_version":"2026.08.1"}],
  "content_digest":{"algorithm":"sha256","hex":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},
  "parser_name":"chatgpt_export","parser_version":"2026.08.1"
}"#;

const PROJECT: &str = r#"{
  "ai_project_id":"018f0000-0000-7000-8000-000000000404", "provider":"chatgpt",
  "title":"Rust notes", "parser_name":"chatgpt_export", "parser_version":"2026.08.1"
}"#;

#[test]
fn import_event_round_trips_the_published_contract_fixture()
-> Result<(), Box<dyn std::error::Error>> {
    let payload: AiArchiveImport = serde_json::from_str(IMPORT)?;
    let event = NormalizedArchiveEvent::archive_imported(payload)?;
    assert_eq!(event.event_type, "ai_archive.archive.imported.v1");
    let round_trip: AiArchiveImport = serde_json::from_value(event.payload)?;
    round_trip.validate()?;
    assert_eq!(
        round_trip.ai_archive_id.to_string(),
        "018f0000-0000-7000-8000-000000000402"
    );
    Ok(())
}

#[test]
fn conversation_event_round_trips_the_published_contract_fixture()
-> Result<(), Box<dyn std::error::Error>> {
    let payload = AiConversationAdded {
        import_provenance: serde_json::from_str::<AiArchiveProvenance>(PROVENANCE)?,
        conversation: serde_json::from_str::<AiConversation>(CONVERSATION)?,
        extensions: Extensions::new(),
    };
    let event = NormalizedArchiveEvent::conversation_added(payload)?;
    assert_eq!(event.event_type, "ai_archive.conversation.added.v1");
    let round_trip: AiConversationAdded = serde_json::from_value(event.payload)?;
    round_trip.validate()?;
    assert_eq!(
        round_trip.import_provenance.ai_archive_id.to_string(),
        "018f0000-0000-7000-8000-000000000402"
    );
    Ok(())
}

#[test]
fn project_event_round_trips_import_provenance_and_content_digest()
-> Result<(), Box<dyn std::error::Error>> {
    let digest = ContentDigest {
        algorithm: DigestAlgorithm::Sha256,
        hex: DigestHex::parse("1111111111111111111111111111111111111111111111111111111111111111")?,
    };
    let payload = AiProjectAdded {
        import_provenance: serde_json::from_str(PROVENANCE)?,
        project: serde_json::from_str::<AiProject>(PROJECT)?,
        content_digest: digest,
        extensions: Extensions::new(),
    };
    let event = NormalizedArchiveEvent::project_added(payload)?;
    assert_eq!(event.event_type, "ai_archive.project.added.v1");
    let round_trip: AiProjectAdded = serde_json::from_value(event.payload)?;
    round_trip.validate()?;
    Ok(())
}

#[test]
fn tombstone_event_round_trips_authoritative_deletion_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let tombstone: AiArchiveTombstone = serde_json::from_str(
        r#"{
      "ai_archive_id":"018f0000-0000-7000-8000-000000000402", "provider":"chatgpt",
      "owner":"user:018f0000-0000-7000-8000-000000000005", "subject":{"subject_kind":"archive"},
      "reason":"provider_deletion_event", "evidence_ref":{"owner_service":"ratatoskr-chatgpt","digest":{"algorithm":"sha256","hex":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"media_type":"application/json","length_bytes":512}, "observed_at":"2026-08-27T06:00:00Z"
    }"#,
    )?;
    let event = NormalizedArchiveEvent::tombstoned(tombstone)?;
    assert_eq!(event.event_type, "ai_archive.subject.tombstoned.v1");
    let round_trip: AiArchiveTombstone = serde_json::from_value(event.payload)?;
    assert_eq!(
        round_trip.subject,
        ratatoskr_ai_archive_contracts::AiArchiveTombstoneSubject::Archive
    );
    Ok(())
}

#[test]
fn user_requested_deletion_event_round_trips() {
    let parsed = serde_json::from_str::<AiArchiveTombstone>(
        r#"{
      "ai_archive_id":"018f0000-0000-7000-8000-000000000402", "provider":"chatgpt",
      "owner":"user:018f0000-0000-7000-8000-000000000005", "subject":{"subject_kind":"conversation","ai_conversation_id":"018f0000-0000-7000-8000-000000000403"},
      "reason":"user_requested", "evidence_ref":{"owner_service":"ratatoskr-chatgpt","digest":{"algorithm":"sha256","hex":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"media_type":"application/json","length_bytes":512}, "observed_at":"2026-08-27T06:00:00Z"
    }"#,
    );
    assert!(
        parsed.is_ok(),
        "the published deletion reason must deserialize: {parsed:?}"
    );

    let event = NormalizedArchiveEvent::tombstoned(parsed.expect("asserted successful parse"))
        .expect("a valid tombstone must encode");
    assert_eq!(event.event_type, "ai_archive.subject.tombstoned.v1");
    let round_trip = serde_json::from_value::<AiArchiveTombstone>(event.payload);
    assert!(round_trip.is_ok(), "encoded payload must round-trip");
}
