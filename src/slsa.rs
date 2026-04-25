//! SLSA Provenance Generation
//!
//! This module generates SLSA (Supply-chain Levels for Software Artifacts) provenance
//! for built artifacts. It produces in-toto attestation bundles with DSSE signatures.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub mod in_toto;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceGenerator {
    builder_id: String,
    build_type: String,
}

impl ProvenanceGenerator {
    pub fn new(builder_id: String) -> Self {
        Self {
            builder_id,
            build_type: "https://memobuild.build/nrelab.io/v1".to_string(),
        }
    }

    pub fn generate_provenance(
        &self,
        source_uri: &str,
        source_digest: &str,
        artifact_uri: &str,
        artifact_digest: &str,
        invocation_params: &InvocationParams,
    ) -> Result<Provenance> {
        let run_details = RunDetails {
            builder: Builder {
                id: self.builder_id.clone(),
            },
            metadata: ProvenanceMetadata {
                invocation_id: uuid::Uuid::new_v4().to_string(),
                started_at: Utc::now(),
                finished_at: Utc::now(),
            },
            environment: invocation_params.environment.clone(),
        };

        let materials = vec![Material {
            uri: source_uri.to_string(),
            digest: source_digest.to_string(),
        }];

        let products = vec![Product {
            uri: artifact_uri.to_string(),
            digest: artifact_digest.to_string(),
        }];

        Ok(Provenance {
            _type: "https://in-toto.io/attestation/v1".to_string(),
            predicate: Predicate {
                build_type: self.build_type.clone(),
                build_definition: BuildDefinition {
                    build_platform: "memobuild".to_string(),
                    invocation_config: invocation_params.clone(),
                    resolved_dependencies: materials.clone(),
                },
                run_details,
            },
            subject: products,
        })
    }

    pub fn sign(&self, provenance: &Provenance) -> Result<Attestation> {
        let payload = serde_json::to_string(provenance)?;
        let signature = in_toto::sign_dsse(&payload)?;
        
        Ok(Attestation {
            payload_type: "application/vnd.in-toto+json".to_string(),
            payload,
            signatures: vec![signature],
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvocationParams {
    pub command: Vec<String>,
    pub environment: Vec<EnvironmentVariable>,
    pub inputs: Vec<InputArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentVariable {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputArtifact {
    pub uri: String,
    pub digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    #[serde(rename = "@type")]
    _type: String,
    predicate: Predicate,
    subject: Vec<Product>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Predicate {
    #[serde(rename = "buildType")]
    build_type: String,
    #[serde(rename = "buildDefinition")]
    build_definition: BuildDefinition,
    #[serde(rename = "runDetails")]
    run_details: RunDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildDefinition {
    #[serde(rename = "buildPlatform")]
    build_platform: String,
    #[serde(rename = "invocationConfig")]
    invocation_config: InvocationParams,
    #[serde(rename = "resolvedDependencies")]
    resolved_dependencies: Vec<Material>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunDetails {
    builder: Builder,
    metadata: ProvenanceMetadata,
    environment: Vec<EnvironmentVariable>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Builder {
    id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceMetadata {
    #[serde(rename = "invocationId")]
    invocation_id: String,
    #[serde(rename = "startedAt")]
    started_at: DateTime<Utc>,
    #[serde(rename = "finishedAt")]
    finished_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Material {
    uri: String,
    digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Product {
    uri: String,
    digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attestation {
    #[serde(rename = "payloadType")]
    payload_type: String,
    payload: String,
    signatures: Vec<Signature>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signature {
    pub keyid: Option<String>,
    pub sig: String,
}

impl Default for InvocationParams {
    fn default() -> Self {
        Self {
            command: vec![],
            environment: vec![],
            inputs: vec![],
        }
    }
}