//! First synthetic `conversations.json` parser contract.

mod decode;
mod input;
mod model;
mod parts;

use std::collections::BTreeSet;

pub use model::{
    ContentPartKind, MessageRole, ParsedContentPart, ParsedConversation, ParsedConversations,
    ParsedMessage, RawRecord,
};

use crate::receipt::AcquisitionMode;
use crate::{ParserId, ParserRegistration};

/// Stable synthetic schema identifier.
pub const SYNTHETIC_SCHEMA_ID: &str = "chatgpt.synthetic.conversations-json";
/// Stable first synthetic parser version.
pub const SYNTHETIC_PARSER_VERSION: &str = "0.1.0";
/// Stable first synthetic parser name.
pub const SYNTHETIC_PARSER_NAME: &str = "synthetic-conversations";

/// Conservative parser error with no source-content disclosure.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SyntheticParserError {
    /// The registry selected a different parser identity.
    #[error("selected parser identity does not match synthetic parser")]
    UnexpectedSelection,
    /// The document does not match the documented synthetic shape.
    #[error("synthetic conversations document is invalid")]
    InvalidDocument,
}

/// Parser for the documented synthetic conversations shape.
#[derive(Debug, Default)]
pub struct SyntheticConversationsParser;

impl SyntheticConversationsParser {
    /// Returns the registry declaration for this parser.
    #[must_use]
    pub fn registration() -> ParserRegistration {
        ParserRegistration {
            id: ParserId {
                name: SYNTHETIC_PARSER_NAME.to_owned(),
                version: SYNTHETIC_PARSER_VERSION.to_owned(),
            },
            modes: vec![AcquisitionMode::ConsumerExport],
            required_signals: BTreeSet::from(["conversations.json".to_owned()]),
        }
    }

    /// Parses bytes after a registry has selected this parser identity.
    ///
    /// # Errors
    ///
    /// Returns a structural error without exposing source content.
    pub fn parse(
        &self,
        source: &[u8],
        selected: &ParserId,
    ) -> Result<ParsedConversations, SyntheticParserError> {
        if selected != &Self::registration().id {
            return Err(SyntheticParserError::UnexpectedSelection);
        }
        decode::parse(source, selected.clone())
    }
}
