//! Verified asset-reference parsing for the synthetic archive grammar.

use serde::Deserialize;
use serde_json::{Map, Value};

use super::parts::record_extra;
use super::{
    AssetAnomaly, AssetAvailability, AssetKind, ParsedAsset, RawRecord, SyntheticParserError,
};
use crate::{BlobStore, ExtractedArtifact};

#[derive(Deserialize)]
struct AssetInput {
    id: String,
    kind: String,
    project_id: Option<String>,
    conversation_id: Option<String>,
    display_name: Option<String>,
    archive_path: Option<String>,
    media_type: Option<String>,
    length_bytes: Option<u64>,
    sha256: Option<String>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

pub(super) async fn parse(
    source: Option<&[u8]>,
    artifacts: &[ExtractedArtifact],
    blob_store: &BlobStore,
    raw_records: &mut Vec<RawRecord>,
) -> Result<Vec<ParsedAsset>, SyntheticParserError> {
    let Some(source) = source else {
        return Ok(Vec::new());
    };
    let inputs: Vec<AssetInput> =
        serde_json::from_slice(source).map_err(|_| SyntheticParserError::InvalidDocument)?;
    let mut assets = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.into_iter().enumerate() {
        assets.push(parse_one(input, index, artifacts, blob_store, raw_records).await);
    }
    Ok(assets)
}

async fn parse_one(
    input: AssetInput,
    index: usize,
    artifacts: &[ExtractedArtifact],
    blob_store: &BlobStore,
    raw_records: &mut Vec<RawRecord>,
) -> ParsedAsset {
    let path = format!("/assets/{index}");
    record_extra(raw_records, &path, &input.extra);
    let (availability, blob, anomaly) = verify(&input, artifacts, blob_store).await;
    ParsedAsset {
        external_id: input.id,
        kind: asset_kind(&input.kind),
        project_external_id: input.project_id,
        conversation_external_id: input.conversation_id,
        display_name: input.display_name,
        media_type: input.media_type,
        availability,
        blob,
        anomaly,
        provider_metadata: Value::Object(input.extra),
    }
}

async fn verify(
    input: &AssetInput,
    artifacts: &[ExtractedArtifact],
    blob_store: &BlobStore,
) -> (
    AssetAvailability,
    Option<ratatoskr_identifiers::BlobRef>,
    Option<AssetAnomaly>,
) {
    let Some(archive_path) = input.archive_path.as_deref() else {
        return (AssetAvailability::Missing, None, None);
    };
    let matching = artifacts
        .iter()
        .filter(|artifact| artifact.provenance.entry_path == archive_path)
        .collect::<Vec<_>>();
    let [artifact] = matching.as_slice() else {
        let anomaly = if matching.is_empty() {
            AssetAnomaly::MissingArtifact
        } else {
            AssetAnomaly::AmbiguousArtifact
        };
        return (AssetAvailability::Quarantined, None, Some(anomaly));
    };
    if artifact.quarantined {
        return (
            AssetAvailability::Quarantined,
            None,
            Some(AssetAnomaly::ExtractedArtifactQuarantined),
        );
    }
    if blob_store.verify(&artifact.blob).await.is_err() {
        return (
            AssetAvailability::Quarantined,
            None,
            Some(AssetAnomaly::BlobVerificationFailed),
        );
    }
    let Some(declared_digest) = input.sha256.as_deref() else {
        return (
            AssetAvailability::Quarantined,
            None,
            Some(AssetAnomaly::InvalidDeclaration),
        );
    };
    if declared_digest != artifact.blob.digest.hex.as_str() {
        return (
            AssetAvailability::Quarantined,
            None,
            Some(AssetAnomaly::DigestMismatch),
        );
    }
    let Some(declared_length) = input.length_bytes else {
        return (
            AssetAvailability::Quarantined,
            None,
            Some(AssetAnomaly::InvalidDeclaration),
        );
    };
    if declared_length != artifact.blob.length_bytes {
        return (
            AssetAvailability::Quarantined,
            None,
            Some(AssetAnomaly::LengthMismatch),
        );
    }
    let Some(declared_media_type) = input.media_type.as_deref() else {
        return (
            AssetAvailability::Quarantined,
            None,
            Some(AssetAnomaly::InvalidDeclaration),
        );
    };
    if declared_media_type != artifact.blob.media_type.as_str() {
        return (
            AssetAvailability::Quarantined,
            None,
            Some(AssetAnomaly::MediaTypeMismatch),
        );
    }
    (
        AssetAvailability::Verified,
        Some(artifact.blob.clone()),
        None,
    )
}

fn asset_kind(kind: &str) -> AssetKind {
    match kind {
        "uploaded" => AssetKind::Uploaded,
        "generated" => AssetKind::Generated,
        _ => AssetKind::Unknown,
    }
}
