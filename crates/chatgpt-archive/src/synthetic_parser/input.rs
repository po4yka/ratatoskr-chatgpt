use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Deserialize)]
pub(super) struct ConversationInput {
    pub(super) id: String,
    pub(super) title: Option<String>,
    pub(super) create_time: Option<f64>,
    pub(super) update_time: Option<f64>,
    pub(super) mapping: BTreeMap<String, MappingInput>,
    #[serde(flatten)]
    pub(super) extra: Map<String, Value>,
}

#[derive(Deserialize)]
pub(super) struct MappingInput {
    pub(super) id: Option<String>,
    pub(super) parent: Option<String>,
    pub(super) message: Option<MessageInput>,
    #[serde(flatten)]
    pub(super) extra: Map<String, Value>,
}

#[derive(Deserialize)]
pub(super) struct MessageInput {
    pub(super) id: String,
    pub(super) author: AuthorInput,
    pub(super) create_time: Option<f64>,
    pub(super) update_time: Option<f64>,
    #[serde(default)]
    pub(super) metadata: MetadataInput,
    pub(super) content: ContentInput,
    #[serde(flatten)]
    pub(super) extra: Map<String, Value>,
}

#[derive(Deserialize)]
pub(super) struct AuthorInput {
    pub(super) role: String,
    #[serde(flatten)]
    pub(super) extra: Map<String, Value>,
}

#[derive(Default, Deserialize)]
pub(super) struct MetadataInput {
    pub(super) model_slug: Option<String>,
    #[serde(flatten)]
    pub(super) extra: Map<String, Value>,
}

#[derive(Deserialize)]
pub(super) struct ContentInput {
    pub(super) parts: Vec<Value>,
    #[serde(flatten)]
    pub(super) extra: Map<String, Value>,
}
