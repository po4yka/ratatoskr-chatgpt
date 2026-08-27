//! Normalized records emitted by the synthetic conversations parser.

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
    /// Unconsumed provider-shaped values with their source locations.
    pub raw_records: Vec<RawRecord>,
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
