//! Parser registry selection matrix.

use ratatoskr_chatgpt_archive::{
    AcquisitionMode, ArchiveInventory, ParserId, ParserRegistration, ParserRegistry,
    ParserSelection, RegistryError,
};
use std::collections::BTreeSet;

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
