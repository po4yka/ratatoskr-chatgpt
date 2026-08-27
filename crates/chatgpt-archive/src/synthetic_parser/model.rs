//! Normalized records emitted by the synthetic conversations parser.

use ratatoskr_identifiers::BlobRef;
use serde_json::Value;

use crate::ParserId;

/// A successful parse of one synthetic conversations document.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedConversations {
    /// Schema detected from the parsed document.
    pub schema_id: String,
    /// Parser identity that interpreted the document.
    pub parser: ParserId,
    /// Conversations in source order.
    pub conversations: Vec<ParsedConversation>,
    /// Projects evidenced by the selected archive parser.
    pub projects: Vec<ParsedProject>,
    /// Canvas-like documents evidenced by the selected archive parser.
    pub canvas_documents: Vec<ParsedCanvasDocument>,
    /// File and generated-asset references evidenced by the selected parser.
    pub assets: Vec<ParsedAsset>,
    /// Unconsumed provider-shaped values with their source locations.
    pub raw_records: Vec<RawRecord>,
}

/// A project projection evidenced by an archive.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedProject {
    /// Provider project identifier.
    pub external_id: String,
    /// Provider title when present.
    pub title: Option<String>,
    /// Provider description when present.
    pub description: Option<String>,
    /// Ordered project instructions and system prompts.
    pub instructions: Vec<ParsedInstruction>,
    /// Provider conversation identifiers linked by the project evidence.
    pub conversation_external_ids: Vec<String>,
    /// Provider asset identifiers linked by the project evidence.
    pub asset_external_ids: Vec<String>,
    /// Unconsumed project fields.
    pub provider_metadata: Value,
}

/// An ordered instruction or system prompt observed on a project.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedInstruction {
    /// Stable provider instruction identifier when supplied.
    pub external_id: Option<String>,
    /// Zero-based provider order within the project.
    pub ordinal: usize,
    /// Whether the provider identified this as an instruction or system prompt.
    pub kind: InstructionKind,
    /// Inert provider-supplied content.
    pub content: Value,
    /// Unconsumed instruction fields.
    pub provider_metadata: Value,
}

/// A recognized kind of project instruction evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstructionKind {
    /// Project-authored instruction text.
    Instruction,
    /// A provider system prompt.
    SystemPrompt,
    /// A future provider variant retained as raw evidence.
    Unknown,
}

/// A Canvas-like document observed in an archive.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedCanvasDocument {
    /// Provider document identifier.
    pub external_id: String,
    /// Linked provider project identifier when present.
    pub project_external_id: Option<String>,
    /// Linked provider conversation identifier when present.
    pub conversation_external_id: Option<String>,
    /// Ordered inert document content supplied by the provider.
    pub content: Vec<Value>,
    /// Unconsumed document fields.
    pub provider_metadata: Value,
}

/// A file or generated-asset reference observed in an archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAsset {
    /// Provider asset identifier.
    pub external_id: String,
    /// Provider asset provenance.
    pub kind: AssetKind,
    /// Linked provider project identifier when present.
    pub project_external_id: Option<String>,
    /// Linked provider conversation identifier when present.
    pub conversation_external_id: Option<String>,
    /// Provider filename or display label when present.
    pub display_name: Option<String>,
    /// Provider-declared media type when present.
    pub media_type: Option<String>,
    /// Current evidence availability.
    pub availability: AssetAvailability,
    /// Usable verified reference only.
    pub blob: Option<BlobRef>,
    /// Stable structural anomaly code when unavailable due to a failed check.
    pub anomaly: Option<AssetAnomaly>,
    /// Unconsumed asset fields.
    pub provider_metadata: Value,
}

/// Provider distinction between uploaded and generated assets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    /// User-uploaded provider asset.
    Uploaded,
    /// Provider-generated asset.
    Generated,
    /// Unrecognized provider kind retained as an unavailable reference.
    Unknown,
}

/// Whether local archive bytes were verified for an asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetAvailability {
    /// The export names a reference but supplies no archive bytes.
    Missing,
    /// Bytes or their declaration failed a security or integrity check.
    Quarantined,
    /// A verified `BlobRef` names locally archived bytes.
    Verified,
}

/// Non-sensitive reason an asset could not be associated with usable bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetAnomaly {
    /// No extracted artifact matched the declared archive path.
    MissingArtifact,
    /// The archive extractor quarantined the candidate artifact.
    ExtractedArtifactQuarantined,
    /// `BlobRef` verification failed closed.
    BlobVerificationFailed,
    /// Provider digest does not equal the verified `BlobRef` digest.
    DigestMismatch,
    /// Provider byte length does not equal the verified `BlobRef` length.
    LengthMismatch,
    /// Provider media type does not equal the verified `BlobRef` media type.
    MediaTypeMismatch,
    /// The provider declaration lacked a required structural field.
    InvalidDeclaration,
    /// More than one extracted artifact named the declared archive path.
    AmbiguousArtifact,
}

/// A normalized conversation projection.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedConversation {
    /// Provider conversation identifier.
    pub external_id: String,
    /// Provider title when present.
    pub title: Option<String>,
    /// Provider creation time as epoch seconds when present.
    pub created_at_epoch_seconds: Option<f64>,
    /// Provider update time as epoch seconds when present.
    pub updated_at_epoch_seconds: Option<f64>,
    /// Unconsumed conversation fields.
    pub provider_metadata: Value,
    /// Messages in deterministic mapping-key order.
    pub messages: Vec<ParsedMessage>,
}

/// A normalized message projection.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedMessage {
    /// Provider message identifier.
    pub external_id: String,
    /// Provider parent-message identifier when present.
    pub parent_external_id: Option<String>,
    /// Provider message role.
    pub role: MessageRole,
    /// Provider creation time as epoch seconds when present.
    pub created_at_epoch_seconds: Option<f64>,
    /// Provider update time as epoch seconds when present.
    pub updated_at_epoch_seconds: Option<f64>,
    /// Provider model slug when present.
    pub model_slug: Option<String>,
    /// Unconsumed message fields.
    pub provider_metadata: Value,
    /// Content parts in provider order.
    pub parts: Vec<ParsedContentPart>,
}

/// A provider message role normalized for the owned schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageRole {
    /// System-authored content.
    System,
    /// User-authored content.
    User,
    /// Assistant-authored content.
    Assistant,
    /// Tool-authored content.
    Tool,
    /// Provider-internal content.
    Internal,
    /// An unrecognized role retained in raw evidence.
    Unknown,
}

/// One heterogeneous content part projection.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedContentPart {
    /// Zero-based position inside the provider message.
    pub ordinal: usize,
    /// Type suitable for the owned content-parts schema.
    pub kind: ContentPartKind,
    /// Original JSON value of the part.
    pub payload: Value,
}

/// A content-parts schema type recognized by the synthetic parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentPartKind {
    /// Plain text.
    Text,
    /// A tool invocation reference.
    ToolCall,
    /// A tool output reference.
    ToolResult,
    /// An image reference without locally archived bytes.
    Image,
    /// A non-image file reference without locally archived bytes.
    File,
    /// An unrecognized provider part.
    Unknown,
}

/// A lossless provider-shaped value retained for later parser versions.
#[derive(Debug, Clone, PartialEq)]
pub struct RawRecord {
    /// Deterministic JSON-pointer location in the source document.
    pub path: String,
    /// Original JSON value.
    pub payload: Value,
}
