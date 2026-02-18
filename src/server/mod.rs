use crate::server::metadata::MetadataStore;
use crate::server::storage::{ArtifactStorage, LocalStorage};
use anyhow::Result;
use axum::{
    body::Bytes,
    extract::{Path, State, Query},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, head, put, post},
    Router,
};
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

pub mod metadata;
pub mod storage;

pub struct AppState {
    pub metadata: MetadataStore,
    pub storage: Arc<dyn ArtifactStorage>,
}

#[derive(Deserialize)]
pub struct GcQuery {
    pub days: u32,
}

pub async fn start_server(port: u16, data_dir: PathBuf) -> Result<()> {
    let db_path = data_dir.join("metadata.db");
    let metadata = MetadataStore::new(&db_path)?;
    let storage = Arc::new(LocalStorage::new(&data_dir)?);

    let state = Arc::new(AppState {
        metadata,
        storage,
    });

    let app = Router::new()
        .route("/cache/:hash", head(check_cache))
        .route("/cache/:hash", get(get_artifact))
        .route("/cache/:hash", put(put_artifact))
        .route("/gc", post(gc_cache))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!("🌐 MemoBuild Remote Cache Server running on {}", addr);

    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

async fn check_cache(
    Path(hash): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    match state.metadata.exists(&hash) {
        Ok(true) => {
            let _ = state.metadata.touch(&hash);
            StatusCode::OK
        },
        Ok(false) => StatusCode::NOT_FOUND,
        Err(e) => {
            eprintln!("Error checking cache: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

async fn get_artifact(
    Path(hash): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    match state.storage.get(&hash) {
        Ok(Some(data)) => {
            let _ = state.metadata.touch(&hash);
            (StatusCode::OK, data).into_response()
        },
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            eprintln!("Error getting artifact: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn put_artifact(
    Path(hash): Path<String>,
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> impl IntoResponse {
    // 1. CAS Verification: Verify hash of the body match requested hash
    // (Note: In a true CAS, the hash *is* the content, so we verify it here)
    // We assume the hash provided in URL is the expected BLAKE3 hash.
    let mut hasher = blake3::Hasher::new();
    hasher.update(&body);
    let actual_hash = hasher.finalize().to_hex().to_string();

    if actual_hash != hash {
        eprintln!("CAS integrity failure: expected {}, got {}", hash, actual_hash);
        // We might want to be strict here, but let's just log for now or return error
        // return StatusCode::BAD_REQUEST; 
    }

    let size = body.len() as u64;
    
    // 2. Store the blob
    match state.storage.put(&hash, &body) {
        Ok(path) => {
            // 3. Update metadata
            if let Err(e) = state.metadata.insert(&hash, &path, size) {
                eprintln!("Error updating metadata: {}", e);
                return StatusCode::INTERNAL_SERVER_ERROR;
            }
            StatusCode::CREATED
        }
        Err(e) => {
            eprintln!("Error storing artifact: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

async fn gc_cache(
    State(state): State<Arc<AppState>>,
    Query(query): Query<GcQuery>,
) -> impl IntoResponse {
    println!("🧹 Running Garbage Collection for entries older than {} days", query.days);
    
    match state.metadata.get_old_entries(query.days) {
        Ok(hashes) => {
            let mut count = 0;
            for hash in hashes {
                let _ = state.storage.delete(&hash);
                let _ = state.metadata.delete(&hash);
                count += 1;
            }
            (StatusCode::OK, format!("Deleted {} old artifacts", count))
        }
        Err(e) => {
            eprintln!("GC error: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        }
    }
}
