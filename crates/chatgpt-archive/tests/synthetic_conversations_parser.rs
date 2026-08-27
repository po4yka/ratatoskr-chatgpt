//! Synthetic conversations parser contract.

#![expect(
    clippy::expect_used,
    reason = "integration-test assertions need contextual failure messages"
)]
#![expect(
    clippy::panic,
    reason = "the registry-selection helper must fail explicitly in a contract test"
)]

use std::collections::BTreeSet;

use ratatoskr_chatgpt_archive::{
    AcquisitionMode, ArchiveInventory, BlobStore, ContentPartKind, ParserRegistry, ParserSelection,
    SYNTHETIC_PARSER_NAME, SYNTHETIC_PARSER_VERSION, SYNTHETIC_SCHEMA_ID, SyntheticArchiveInput,
    SyntheticConversationsParser,
};

const FIXTURE: &[u8] = include_bytes!("fixtures/synthetic_conversations.json");

fn selected_parser() -> ratatoskr_chatgpt_archive::ParserId {
    let mut registry = ParserRegistry::default();
    registry
        .register(SyntheticConversationsParser::registration())
        .expect("synthetic parser registration must be unique");
    let inventory = ArchiveInventory {
        entries: Vec::new(),
        compressed_bytes: 0,
        decompressed_bytes: 0,
        signals: BTreeSet::from(["conversations.json".to_owned()]),
    };
    match registry.select(&inventory, AcquisitionMode::ConsumerExport) {
        ParserSelection::Selected(parser) => parser,
        other => panic!("synthetic parser must be selected, got {other:?}"),
    }
}

async fn parse_fixture() -> ratatoskr_chatgpt_archive::ParsedConversations {
    let root = tempfile::tempdir().expect("temporary blob root");
    let store = BlobStore::new(root.path()).expect("blob store");
    let artifacts = Vec::new();
    let selected = selected_parser();
    let input =
        SyntheticArchiveInput::new(&selected, FIXTURE, None, None, None, &artifacts, &store);
    SyntheticConversationsParser
        .parse_archive(&input)
        .await
        .expect("synthetic fixture must parse")
}

#[tokio::test]
async fn synthetic_fixture_maps_conversations_messages_and_parts() {
    let parsed = parse_fixture().await;
    assert_eq!(parsed.conversations.len(), 2);
    assert_eq!(parsed.conversations[0].external_id, "conversation-alpha");
    assert_eq!(parsed.conversations[0].messages.len(), 2);
    let user = parsed.conversations[0]
        .messages
        .iter()
        .find(|message| message.external_id == "message-user")
        .expect("user message must be mapped");
    assert_eq!(user.parts[0].kind, ContentPartKind::Text);
    let assistant = parsed.conversations[0]
        .messages
        .iter()
        .find(|message| message.external_id == "message-assistant")
        .expect("assistant message must be mapped");
    assert_eq!(assistant.parts[0].kind, ContentPartKind::ToolCall);
    assert_eq!(assistant.parts[1].kind, ContentPartKind::Image);
    assert_eq!(
        parsed.conversations[1].messages[0].parts[1].kind,
        ContentPartKind::File
    );
}

#[tokio::test]
async fn successful_parse_carries_schema_and_parser_version() {
    let parsed = parse_fixture().await;
    assert_eq!(parsed.schema_id, SYNTHETIC_SCHEMA_ID);
    assert_eq!(parsed.parser.name, SYNTHETIC_PARSER_NAME);
    assert_eq!(parsed.parser.version, SYNTHETIC_PARSER_VERSION);
}

#[tokio::test]
async fn parsing_identical_fixture_is_deterministic() {
    assert_eq!(parse_fixture().await, parse_fixture().await);
}

#[tokio::test]
async fn unknown_fields_and_parts_remain_losslessly_available() {
    let parsed = parse_fixture().await;
    assert!(parsed.raw_records.iter().any(|record| {
        record.path == "/0/conversation_unknown" && record.payload["retained"] == true
    }));
    let assistant = parsed.conversations[0]
        .messages
        .iter()
        .find(|message| message.external_id == "message-assistant")
        .expect("assistant message must be mapped");
    assert_eq!(assistant.parts[2].ordinal, 2);
    assert_eq!(assistant.parts[2].kind, ContentPartKind::Unknown);
    assert_eq!(assistant.parts[2].payload["kind"], "future_part");
    assert!(parsed.raw_records.iter().any(|record| {
        record.path == "/0/mapping/message-assistant/message/message_unknown"
            && record.payload == serde_json::json!(["preserve"])
    }));
}
