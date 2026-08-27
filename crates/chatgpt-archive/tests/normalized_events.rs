//! Published normalized-event conformance and linkage tests.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "synthetic contract fixtures"
)]

use ratatoskr_ai_archive_contracts::{
    AiArchiveProvenance, AiArchiveTombstone, AiConversation, AiConversationAdded, AiProject,
    AiProjectAdded,
};
use ratatoskr_chatgpt_archive::NormalizedArchiveEvent;
use ratatoskr_identifiers::{ContentDigest, DigestAlgorithm, DigestHex, Extensions};

const PROVENANCE: &str = r#"{
  "ai_archive_id":"018f0000-0000-7000-8000-000000000402",
  "provider":"chatgpt", "owner":"user:018f0000-0000-7000-8000-000000000005",
  "source_export":{"owner_service":"ratatoskr-chatgpt","digest":{"algorithm":"sha256","hex":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"media_type":"application/json","length_bytes":512},
  "imported_at":"2026-08-17T10:00:00Z", "parser_name":"chatgpt_export", "parser_version":"2026.08.1"
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
