//! Parser registry selection matrix.

use ratatoskr_chatgpt_archive::{
    AcquisitionMode, ArchiveInventory, ParsedConversations, ParserExecutionError,
    ParserExecutionInput, ParserExecutor, ParserId, ParserRegistration, ParserRegistry,
    ParserSelection, RegistryError,
};
use std::collections::BTreeSet;
use std::sync::Arc;

fn inventory() -> ArchiveInventory {
    ArchiveInventory {
        entries: Vec::new(),
        compressed_bytes: 0,
        decompressed_bytes: 0,
        signals: BTreeSet::from(["conversations.json".to_owned()]),
    }
}
fn parser(name: &str) -> ParserRegistration {
    ParserRegistration {
        id: ParserId {
            name: name.to_owned(),
            version: "1".to_owned(),
        },
        modes: vec![AcquisitionMode::ConsumerExport],
        required_signals: BTreeSet::from(["conversations.json".to_owned()]),
    }
}

fn versioned_parser(version: &str) -> ParserRegistration {
    let mut parser = parser("chatgpt");
    version.clone_into(&mut parser.id.version);
    parser
}

#[derive(Debug)]
struct EmptyParser;

impl ParserExecutor for EmptyParser {
    fn execute(
        &self,
        _input: ParserExecutionInput<'_>,
    ) -> Result<ParsedConversations, ParserExecutionError> {
        Ok(ParsedConversations {
            schema_id: "synthetic.chatgpt.test".to_owned(),
            parser: ParserId {
                name: "chatgpt".to_owned(),
                version: "1.10".to_owned(),
            },
            conversations: Vec::new(),
            projects: Vec::new(),
            canvas_documents: Vec::new(),
            assets: Vec::new(),
            raw_records: Vec::new(),
        })
    }
}

#[test]
fn matching_structure_selects_one_versioned_parser() {
    let mut registry = ParserRegistry::default();
    registry
        .register(parser("chatgpt"))
        .expect("first registration");
    assert!(matches!(
        registry.select(&inventory(), AcquisitionMode::ConsumerExport),
        ParserSelection::Selected(_)
    ));
}
#[test]
fn unsupported_structure_is_explicit() {
    assert_eq!(
        ParserRegistry::default().select(&inventory(), AcquisitionMode::ConsumerExport),
        ParserSelection::Unsupported
    );
}
#[test]
fn overlapping_capabilities_are_ambiguous() {
    let mut registry = ParserRegistry::default();
    registry.register(parser("one")).expect("one");
    registry.register(parser("two")).expect("two");
    assert!(matches!(
        registry.select(&inventory(), AcquisitionMode::ConsumerExport),
        ParserSelection::Ambiguous(_)
    ));
}
#[test]
fn duplicate_identity_is_refused() {
    let mut registry = ParserRegistry::default();
    registry.register(parser("one")).expect("one");
    assert!(matches!(
        registry.register(parser("one")),
        Err(RegistryError::DuplicateIdentity)
    ));
}

#[test]
fn compatible_versions_and_exact_lookup_are_deterministic() {
    let mut forward = ParserRegistry::default();
    let mut reverse = ParserRegistry::default();
    for version in ["1.2", "1.10", "2.0"] {
        forward
            .register_compiled(versioned_parser(version), Arc::new(EmptyParser))
            .expect("unique compiled parser");
    }
    for version in ["2.0", "1.10", "1.2"] {
        reverse
            .register_compiled(versioned_parser(version), Arc::new(EmptyParser))
            .expect("unique compiled parser");
    }
    let expected = vec!["1.2", "1.10", "2.0"];
    for registry in [&forward, &reverse] {
        let compatible =
            registry.compatible_versions(&inventory(), AcquisitionMode::ConsumerExport);
        assert_eq!(
            compatible
                .iter()
                .map(|parser| parser.version.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        let exact = registry
            .find_exact(
                &ParserId {
                    name: "chatgpt".to_owned(),
                    version: "1.10".to_owned(),
                },
                &inventory(),
                AcquisitionMode::ConsumerExport,
            )
            .expect("exact compatible compiled parser resolves");
        let parsed = exact
            .execute(ParserExecutionInput {
                inventory: &inventory(),
                artifacts: &[],
            })
            .expect("compiled parser executes");
        assert_eq!(parsed.parser.version, "1.10");
        assert!(matches!(
            registry.select(&inventory(), AcquisitionMode::ConsumerExport),
            ParserSelection::Ambiguous(identities) if identities.len() == 3
        ));
    }
}
