//! First synthetic `conversations.json` parser contract.

mod assets;
mod decode;
mod input;
mod model;
mod parts;

use std::collections::BTreeSet;

pub use model::{
    AssetAnomaly, AssetAvailability, AssetKind, ContentPartKind, InstructionKind, MessageRole,
    ParsedAsset, ParsedCanvasDocument, ParsedContentPart, ParsedConversation, ParsedConversations,
    ParsedInstruction, ParsedMessage, ParsedProject, RawRecord,
};

use crate::receipt::AcquisitionMode;
use crate::{BlobStore, ExtractedArtifact, ParserId, ParserRegistration};

/// All immutable evidence available to the synthetic archive parser.
#[derive(Debug)]
pub struct SyntheticArchiveInput<'a> {
    selected: &'a ParserId,
    conversations_json: &'a [u8],
    projects_json: Option<&'a [u8]>,
    canvas_json: Option<&'a [u8]>,
    assets_json: Option<&'a [u8]>,
    extracted_artifacts: &'a [ExtractedArtifact],
    blob_store: &'a BlobStore,
}

impl<'a> SyntheticArchiveInput<'a> {
    /// Creates a bounded parser input from already-extracted immutable evidence.
    #[must_use]
    pub fn new(
        selected: &'a ParserId,
        conversations_json: &'a [u8],
        projects_json: Option<&'a [u8]>,
        canvas_json: Option<&'a [u8]>,
        assets_json: Option<&'a [u8]>,
        extracted_artifacts: &'a [ExtractedArtifact],
        blob_store: &'a BlobStore,
    ) -> Self {
        Self {
            selected,
            conversations_json,
            projects_json,
            canvas_json,
            assets_json,
            extracted_artifacts,
            blob_store,
        }
    }
}

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

    /// Parses archive evidence after registry selection.
    ///
    /// # Errors
    ///
    /// Returns a structural error without exposing source content.
    pub async fn parse_archive(
        &self,
        input: &SyntheticArchiveInput<'_>,
    ) -> Result<ParsedConversations, SyntheticParserError> {
        if input.selected != &Self::registration().id {
            return Err(SyntheticParserError::UnexpectedSelection);
        }
        let mut parsed = decode::parse_archive(
            input.conversations_json,
            input.projects_json,
            input.canvas_json,
            input.selected.clone(),
        )?;
        parsed.assets = assets::parse(
            input.assets_json,
            input.extracted_artifacts,
            input.blob_store,
            &mut parsed.raw_records,
        )
        .await?;
        Ok(parsed)
    }
}
