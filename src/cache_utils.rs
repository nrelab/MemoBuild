use blake3;

pub const CHUNK_SIZE: usize = 1024 * 1024; // 1MB chunks

pub struct ArtifactLayer {
    pub hash: String,
    pub data: Vec<u8>,
}

/// Split an artifact into content-addressed chunks
pub fn split_artifact(data: &[u8]) -> Vec<ArtifactLayer> {
    let mut layers = Vec::new();
    for chunk in data.chunks(CHUNK_SIZE) {
        let hash = blake3::hash(chunk).to_hex().to_string();
        layers.push(ArtifactLayer {
            hash,
            data: chunk.to_vec(),
        });
    }
    layers
}

/// Merge chunks back into a single artifact
pub fn merge_artifact(layers: Vec<Vec<u8>>) -> Vec<u8> {
    let mut data = Vec::with_capacity(layers.iter().map(|l| l.len()).sum());
    for layer in layers {
        data.extend_from_slice(&layer);
    }
    data
}
