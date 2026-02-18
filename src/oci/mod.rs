pub mod layer;
pub mod config;
pub mod manifest;
pub mod utils;
pub mod registry;

use anyhow::Result;
use std::fs;
use std::path::PathBuf;
use crate::graph::BuildGraph;
use crate::oci::manifest::{OCIManifest, OCIDescriptor, OCIIndex};

pub fn export_image(graph: &BuildGraph, image_name: &str) -> Result<PathBuf> {
    let output_dir = PathBuf::from(".memobuild-output").join(image_name.replace(':', "-"));
    fs::create_dir_all(&output_dir)?;
    
    // 1. Create layers
    let mut layers = Vec::new();
    for node in &graph.nodes {
        // Only export nodes that actually produced something or are dirty
        // In this demo, we export all for simplicity
        let layer_info = layer::create_layer_tar(&output_dir, node)?;
        layers.push(layer_info);
    }
    
    // 2. Create config
    let oci_config = config::create_config(graph, &layers);
    let config_json = serde_json::to_string_pretty(&oci_config)?;
    let config_digest = format!("sha256:{}", utils::sha256_string(&config_json));
    
    let blobs_dir = output_dir.join("blobs").join("sha256");
    fs::create_dir_all(&blobs_dir)?;
    fs::write(blobs_dir.join(&config_digest[7..]), &config_json)?;
    
    // 3. Create manifest
    let manifest = OCIManifest {
        schema_version: 2,
        media_type: "application/vnd.oci.image.manifest.v1+json".to_string(),
        config: OCIDescriptor {
            media_type: "application/vnd.oci.image.config.v1+json".to_string(),
            digest: config_digest,
            size: config_json.len() as u64,
        },
        layers: layers.iter().map(|l| OCIDescriptor {
            media_type: "application/vnd.oci.image.layer.v1.tar+gzip".to_string(),
            digest: l.digest.clone(),
            size: l.size,
        }).collect(),
    };
    
    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    let manifest_digest = format!("sha256:{}", utils::sha256_string(&manifest_json));
    fs::write(blobs_dir.join(&manifest_digest[7..]), &manifest_json)?;
    
    // 4. Create index.json
    let index = OCIIndex {
        schema_version: 2,
        manifests: vec![OCIDescriptor {
            media_type: "application/vnd.oci.image.manifest.v1+json".to_string(),
            digest: manifest_digest,
            size: manifest_json.len() as u64,
        }],
    };
    fs::write(output_dir.join("index.json"), serde_json::to_string_pretty(&index)?)?;
    
    // 5. Create oci-layout
    fs::write(output_dir.join("oci-layout"), r#"{"imageLayoutVersion": "1.0.0"}"#)?;
    
    println!("✅ OCI Image exported to: {}", output_dir.display());
    Ok(output_dir)
}
