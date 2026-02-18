use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use crate::oci::manifest::OCIManifest;

pub struct RegistryClient {
    client: Client,
    base_url: String, // e.g., https://index.docker.io/v2
    repo: String,     // e.g., library/ubuntu
    token: Option<String>,
}

impl RegistryClient {
    pub fn new(registry: &str, repo: &str) -> Self {
        let base_url = if registry.contains("://") {
            format!("{}/v2", registry)
        } else {
            format!("https://{}/v2", registry)
        };

        Self {
            client: Client::new(),
            base_url,
            repo: repo.to_string(),
            token: None,
        }
    }

    pub fn set_token(&mut self, token: &str) {
        self.token = Some(token.to_string());
    }

    /// Push an OCI layout directory to the registry
    pub fn push(&self, layout_dir: &Path) -> Result<()> {
        println!("🚀 Pushing image to {}/{}...", self.base_url, self.repo);

        // 1. Read index.json to find the manifest
        let index_path = layout_dir.join("index.json");
        let index_content = fs::read_to_string(&index_path)?;
        let index: serde_json::Value = serde_json::from_str(&index_content)?;
        let manifest_digest = index["manifests"][0]["digest"].as_str()
            .context("No manifest found in index.json")?;

        // 2. Read the manifest to find layers
        let manifest_path = layout_dir.join("blobs").join("sha256").join(&manifest_digest[7..]);
        let manifest_content = fs::read_to_string(&manifest_path)?;
        let manifest: OCIManifest = serde_json::from_str(&manifest_content)?;

        // 3. Push layers
        for layer in &manifest.layers {
            let layer_path = layout_dir.join("blobs").join("sha256").join(&layer.digest[7..]);
            self.upload_blob(&layer.digest, &layer_path)?;
        }

        // 4. Push config
        let config_path = layout_dir.join("blobs").join("sha256").join(&manifest.config.digest[7..]);
        self.upload_blob(&manifest.config.digest, &config_path)?;

        // 5. Push manifest
        self.upload_manifest(&manifest_digest, &manifest_content)?;

        println!("✅ Image pushed successfully!");
        Ok(())
    }

    fn upload_blob(&self, digest: &str, path: &Path) -> Result<()> {
        // Check if blob exists first (cross-repo mount or just skip)
        if self.blob_exists(digest)? {
            println!("   (skip blob: {} already exists)", &status_hash(digest));
            return Ok(());
        }

        println!("   📤 Uploading blob: {}...", status_hash(digest));

        // Initiating upload
        let url = format!("{}/{}/blobs/uploads/", self.base_url, self.repo);
        let mut rb = self.client.post(&url);
        if let Some(ref t) = self.token {
            rb = rb.bearer_auth(t);
        }
        
        let resp = rb.send()?;
        if !resp.status().is_success() {
            anyhow::bail!("Failed to initiate blob upload: {}", resp.status());
        }

        let location = resp.headers().get("Location")
            .context("No Location header in upload initiation")?
            .to_str()?;

        // Perform the upload (monolithic for simplicity)
        let upload_url = if location.contains("://") {
            format!("{}&digest={}", location, digest)
        } else {
            // Some registries return relative paths
            format!("{}?digest={}", location, digest) // This is oversimplified, usually it's more complex
        };

        // Re-construct the URL if it's relative
        let final_url = if location.starts_with('/') {
            // Extract host from base_url
            let host = self.base_url.split("/v2").next().unwrap();
            format!("{}{}&digest={}", host, location, digest)
        } else {
            format!("{}&digest={}", location, digest)
        };

        let file_content = fs::read(path)?;
        let mut rb = self.client.put(&final_url)
            .body(file_content);
        if let Some(ref t) = self.token {
            rb = rb.bearer_auth(t);
        }

        let resp = rb.send()?;
        if !resp.status().is_success() {
            anyhow::bail!("Failed to upload blob: {}", resp.status());
        }

        Ok(())
    }

    fn blob_exists(&self, digest: &str) -> Result<bool> {
        let url = format!("{}/{}/blobs/{}", self.base_url, self.repo, digest);
        let mut rb = self.client.head(&url);
        if let Some(ref t) = self.token {
            rb = rb.bearer_auth(t);
        }
        let resp = rb.send()?;
        Ok(resp.status().is_success())
    }

    fn upload_manifest(&self, digest: &str, content: &str) -> Result<()> {
        println!("   📜 Uploading manifest: {}...", status_hash(digest));
        let url = format!("{}/{}/manifests/latest", self.base_url, self.repo); // Using 'latest' tag
        
        let mut rb = self.client.put(&url)
            .header("Content-Type", "application/vnd.oci.image.manifest.v1+json")
            .body(content.to_string());
        
        if let Some(ref t) = self.token {
            rb = rb.bearer_auth(t);
        }

        let resp = rb.send()?;
        if !resp.status().is_success() {
            anyhow::bail!("Failed to upload manifest: {}", resp.status());
        }

        Ok(())
    }
}

fn status_hash(digest: &str) -> &str {
    if digest.len() > 15 {
        &digest[7..15]
    } else {
        digest
    }
}
