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
use crate::{
    ArtifactProvenance, BlobStore, EntryKind, ExtractedArtifact, ParserArtifactEvidence,
    ParserExecutionError, ParserExecutionInput, ParserExecutor, ParserFuture, ParserId,
    ParserRegistration,
};

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

impl ParserExecutor for SyntheticConversationsParser {
    fn execute<'a>(&'a self, input: ParserExecutionInput<'a>) -> ParserFuture<'a> {
        Box::pin(async move { self.execute_registered(&input).await })
    }
}

impl SyntheticConversationsParser {
    /// The registered path: the archive's entries are already in memory as evidence, so a
    /// declared file is stored (idempotently, by digest) and verified through the same code
    /// as [`Self::parse_archive`] instead of being refused.
    async fn execute_registered(
        &self,
        input: &ParserExecutionInput<'_>,
    ) -> Result<ParsedConversations, ParserExecutionError> {
        let evidence = |name: &str| -> Option<&[u8]> {
            input
                .artifacts
                .iter()
                .find(|artifact: &&ParserArtifactEvidence| {
                    artifact.path == name && !artifact.quarantined
                })
                .map(|artifact| artifact.bytes.as_ref())
        };
        let conversations = evidence("conversations.json").ok_or(ParserExecutionError::Failed)?;
        let assets_json = evidence("assets.json");
        let extracted = if assets_json.is_some() {
            extract_entries(input).await?
        } else {
            Vec::new()
        };
        let selected = Self::registration().id;
        self.parse_archive(&SyntheticArchiveInput::new(
            &selected,
            conversations,
            evidence("projects.json"),
            evidence("canvas.json"),
            assets_json,
            &extracted,
            input.blobs,
        ))
        .await
        .map_err(|_| ParserExecutionError::Failed)
    }
}

/// Stores each entry of the archive by digest and returns the references a declared file is
/// verified against. A blob store keyed by digest makes this the same object the importer
/// registers later, so nothing new is retained.
async fn extract_entries(
    input: &ParserExecutionInput<'_>,
) -> Result<Vec<ExtractedArtifact>, ParserExecutionError> {
    let mut extracted = Vec::with_capacity(input.artifacts.len());
    for artifact in input.artifacts {
        let kind = input
            .inventory
            .entries
            .iter()
            .find(|entry| entry.path == artifact.path)
            .map_or(EntryKind::Unknown, |entry| entry.kind);
        let blob = input
            .blobs
            .store(
                crate::archive::media_type(kind),
                futures_util::stream::iter([Ok::<_, std::io::Error>(artifact.bytes.clone())]),
            )
            .await
            .map_err(|_| ParserExecutionError::Failed)?;
        extracted.push(ExtractedArtifact {
            blob,
            provenance: ArtifactProvenance {
                raw_archive_digest: input.raw_archive.digest.hex.as_str().to_owned(),
                entry_path: artifact.path.clone(),
            },
            quarantined: artifact.quarantined,
        });
    }
    Ok(extracted)
}

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
