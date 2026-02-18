use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteCacheEntry {
    pub key: String,
    pub size: u64,
}

#[async_trait]
pub trait RemoteCache: Send + Sync {
    async fn has(&self, hash: &str) -> Result<bool>;
    async fn get(&self, hash: &str) -> Result<Option<Vec<u8>>>;
    async fn put(&self, hash: &str, data: &[u8]) -> Result<()>;
    async fn report_analytics(&self, dirty: u32, cached: u32, duration_ms: u64) -> Result<()>;
}

pub struct HttpRemoteCache {
    base_url: String,
    client: Client,
}

impl HttpRemoteCache {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            client: Client::new(),
        }
    }
}

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::{Read, Write};

#[async_trait]
impl RemoteCache for HttpRemoteCache {
    async fn has(&self, hash: &str) -> Result<bool> {
        let url = format!("{}/cache/{}", self.base_url, hash);
        let resp = self.client.head(&url).send().await?;
        Ok(resp.status().is_success())
    }

    async fn get(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        let url = format!("{}/cache/{}", self.base_url, hash);
        let resp = self.client.get(&url).send().await?;

        if resp.status().is_success() {
            let compressed_data = resp.bytes().await?;

            // Decompress
            let mut decoder = GzDecoder::new(&compressed_data[..]);
            let mut decompressed_data = Vec::new();
            decoder.read_to_end(&mut decompressed_data)?;

            Ok(Some(decompressed_data))
        } else if resp.status() == 404 {
            Ok(None)
        } else {
            anyhow::bail!("Remote cache error: {}", resp.status());
        }
    }

    async fn put(&self, hash: &str, data: &[u8]) -> Result<()> {
        // Incremental Layer Update: check if exists before uploading
        if self.has(hash).await? {
            println!("   (skip upload: remote already has {})", &hash[..8]);
            return Ok(());
        }

        let url = format!("{}/cache/{}", self.base_url, hash);

        // Build artifact compression
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data)?;
        let compressed_data = encoder.finish()?;

        let resp = self.client.put(&url).body(compressed_data).send().await?;

        if !resp.status().is_success() {
            anyhow::bail!("Failed to upload to remote cache: {}", resp.status());
        }
        Ok(())
    }

    async fn report_analytics(&self, dirty: u32, cached: u32, duration_ms: u64) -> Result<()> {
        let url = format!("{}/analytics", self.base_url);
        let data = serde_json::json!({
            "dirty": dirty,
            "cached": cached,
            "duration_ms": duration_ms
        });

        let resp = self.client.post(&url).json(&data).send().await?;

        if !resp.status().is_success() {
            eprintln!("Failed to report analytics: {}", resp.status());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    // Note: Integration tests would go here, but they require a running server.
    // For unit tests, we'd need to mock the HTTP client.
}
