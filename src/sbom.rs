//! SBOM Generation (Software Bill of Materials)
//!
//! This module generates CycloneDX SBOMs for built OCI images.
//! It lists all dependencies and their content hashes.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SbomGenerator {
    component_type: String,
    supplier: Option<String>,
}

impl SbomGenerator {
    pub fn new(supplier: Option<String>) -> Self {
        Self {
            component_type: "application".to_string(),
            supplier,
        }
    }

    pub fn generate_sbom(
        &self,
        image_name: &str,
        image_digest: &str,
        layers: &[LayerInfo],
        dependencies: &[Dependency],
    ) -> Result<Sbom> {
        let components: Vec<Component> = dependencies
            .iter()
            .map(|dep| Component {
                r#type: "library".to_string(),
                name: dep.name.clone(),
                version: dep.version.clone(),
                purl: Some(dep.purl.clone()),
                hash: dep.sha256.clone(),
                licenses: dep.licenses.clone(),
            })
            .collect();

        let metadata = Metadata {
            timestamp: chrono::Utc::now().to_rfc3339(),
            tools: vec![Tool {
                name: "memobuild".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            }],
            component: Component {
                r#type: "application".to_string(),
                name: image_name.to_string(),
                version: image_digest[..12].to_string(),
                purl: Some(format!("oci://{}@{}", image_name, image_digest)),
                hash: image_digest.to_string(),
                licenses: vec![],
            },
        };

        Ok(Sbom {
            bom_format: "CycloneDX".to_string(),
            spec_version: "1.5".to_string(),
            serial_number: format!("urn:uuid:{}", uuid::Uuid::new_v4()),
            version: 1,
            metadata,
            components,
            dependencies: vec![],
        })
    }

    pub fn generate_from_dockerfile(&self, dockerfile: &str) -> Result<Sbom> {
        // Parse Dockerfile and extract COPY commands
        let mut components = vec![];

        for line in dockerfile.lines() {
            let line = line.trim();
            if line.starts_with("COPY ") {
                // Extract source files
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let source = parts[1];
                    components.push(Component {
                        r#type: "file".to_string(),
                        name: source.to_string(),
                        version: "1.0.0".to_string(),
                        purl: None,
                        hash: format!("sha256:{}", "placeholder"),
                        licenses: vec![],
                    });
                }
            }
        }

        let metadata = Metadata {
            timestamp: chrono::Utc::now().to_rfc3339(),
            tools: vec![Tool {
                name: "memobuild".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            }],
            component: Component {
                r#type: "application".to_string(),
                name: "dockerfile".to_string(),
                version: "1.0.0".to_string(),
                purl: None,
                hash: "sha256:placeholder".to_string(),
                licenses: vec![],
            },
        };

        Ok(Sbom {
            bom_format: "CycloneDX".to_string(),
            spec_version: "1.5".to_string(),
            serial_number: format!("urn:uuid:{}", uuid::Uuid::new_v4()),
            version: 1,
            metadata,
            components,
            dependencies: vec![],
        })
    }

    pub fn to_json(&self, sbom: &Sbom) -> Result<String> {
        serde_json::to_string_pretty(sbom)
            .map_err(|e| anyhow::anyhow!("Failed to serialize SBOM: {}", e))
    }

    pub fn to_xml(&self, sbom: &Sbom) -> Result<String> {
        // Simple XML serialization
        let mut xml = String::new();
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        xml.push_str("<bom serialNumber=\"");
        xml.push_str(&sbom.serial_number);
        xml.push_str("\" version=\"1\" xmlns=\"http://cyclonedx.org/schema/bom/1.5\">\n");

        // Metadata
        xml.push_str("  <metadata>\n");
        xml.push_str("    <timestamp>");
        xml.push_str(&sbom.metadata.timestamp);
        xml.push_str("</timestamp>\n");
        xml.push_str("    <tools>\n");
        for tool in &sbom.metadata.tools {
            xml.push_str("      <tool>\n");
            xml.push_str("        <name>");
            xml.push_str(&tool.name);
            xml.push_str("</name>\n");
            xml.push_str("        <version>");
            xml.push_str(&tool.version);
            xml.push_str("</version>\n");
            xml.push_str("      </tool>\n");
        }
        xml.push_str("    </tools>\n");
        xml.push_str("  </metadata>\n");

        xml.push_str("</bom>\n");

        Ok(xml)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sbom {
    #[serde(rename = "bomFormat")]
    bom_format: String,
    #[serde(rename = "specVersion")]
    spec_version: String,
    #[serde(rename = "serialNumber")]
    serial_number: String,
    version: i32,
    metadata: Metadata,
    components: Vec<Component>,
    dependencies: Vec<Dependency>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    timestamp: String,
    tools: Vec<Tool>,
    component: Component,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    name: String,
    version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
    #[serde(rename = "type")]
    r#type: String,
    name: String,
    version: String,
    purl: Option<String>,
    #[serde(rename = "hash")]
    hash: String,
    licenses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    pub name: String,
    pub version: String,
    pub purl: String,
    pub sha256: String,
    pub licenses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerInfo {
    pub digest: String,
    pub size: u64,
    pub media_type: String,
}

/// CLI for SBOM generation
pub mod cli {
    use super::*;

    pub fn generate_cmd(image: &str, output: &str) -> Result<()> {
        let generator = SbomGenerator::new(Some("NRELabs".to_string()));

        // In production, would fetch actual layers from registry
        let layers = vec![];
        let dependencies = vec![];

        let sbom = generator.generate_sbom(image, "sha256:abc123", &layers, &dependencies)?;
        let json = generator.to_json(&sbom)?;

        if output == "-" {
            println!("{}", json);
        } else {
            std::fs::write(output, &json)?;
            println!("SBOM written to {}", output);
        }

        Ok(())
    }
}
