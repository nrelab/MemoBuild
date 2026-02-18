use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct OCIManifest {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub config: OCIDescriptor,
    pub layers: Vec<OCIDescriptor>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OCIDescriptor {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub digest: String,
    pub size: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OCIIndex {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    pub manifests: Vec<OCIDescriptor>,
}
