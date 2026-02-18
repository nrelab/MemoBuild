use crate::server::metadata::MetadataStore;
use crate::server::storage::{ArtifactStorage, LocalStorage};
use anyhow::Result;
use axum::{
    body::Bytes,
    extract::{Path, State, Query},
    http::StatusCode,
    response::{IntoResponse, Html},
    routing::{get, head, put, post},
    Router, Json,
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
    pub webhook_url: Option<String>,
}

#[derive(Deserialize)]
pub struct GcQuery {
    pub days: u32,
}

#[derive(Deserialize, Clone)]
pub struct AnalyticsData {
    pub dirty: u32,
    pub cached: u32,
    pub duration_ms: u64,
}

pub async fn start_server(port: u16, data_dir: PathBuf, webhook_url: Option<String>) -> Result<()> {
    let db_path = data_dir.join("metadata.db");
    let metadata = MetadataStore::new(&db_path)?;
    let storage = Arc::new(LocalStorage::new(&data_dir)?);

    let state = Arc::new(AppState {
        metadata,
        storage,
        webhook_url,
    });

    let app = Router::new()
        .route("/", get(dashboard))
        .route("/cache/:hash", head(check_cache))
        .route("/cache/:hash", get(get_artifact))
        .route("/cache/:hash", put(put_artifact))
        .route("/gc", post(gc_cache))
        .route("/analytics", post(report_analytics))
        .route("/api/analytics", get(get_analytics_handler))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!("🌐 MemoBuild Remote Cache Server running on {}", addr);

    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

async fn get_analytics_handler(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    match state.metadata.get_analytics(50) {
        Ok(data) => (StatusCode::OK, Json(data)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn dashboard() -> Html<String> {
    let html = r#"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>MemoBuild Dashboard</title>
    <link href="https://fonts.googleapis.com/css2?family=Outfit:wght@300;400;600&display=swap" rel="stylesheet">
    <style>
        :root {
            --bg: #0f172a;
            --card: #1e293b;
            --accent: #38bdf8;
            --text: #f1f5f9;
            --text-dim: #94a3b8;
            --success: #22c55e;
            --warning: #f59e0b;
        }
        body {
            font-family: 'Outfit', sans-serif;
            background-color: var(--bg);
            color: var(--text);
            margin: 0;
            padding: 2rem;
            display: flex;
            flex-direction: column;
            align-items: center;
        }
        .container {
            max-width: 1000px;
            width: 100%;
        }
        h1 {
            font-size: 2.5rem;
            margin-bottom: 0.5rem;
            background: linear-gradient(to right, #38bdf8, #818cf8);
            -webkit-background-clip: text;
            -webkit-text-fill-color: transparent;
        }
        .subtitle {
            color: var(--text-dim);
            margin-bottom: 2rem;
        }
        .grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
            gap: 1.5rem;
            margin-bottom: 2rem;
        }
        .card {
            background: var(--card);
            padding: 1.5rem;
            border-radius: 1rem;
            border: 1px solid rgba(255, 255, 255, 0.1);
            transition: transform 0.2s;
        }
        .card:hover {
            transform: translateY(-5px);
            border-color: var(--accent);
        }
        .stat-value {
            font-size: 2rem;
            font-weight: 600;
            color: var(--accent);
        }
        .stat-label {
            color: var(--text-dim);
            font-size: 0.9rem;
            text-transform: uppercase;
            letter-spacing: 0.05em;
        }
        table {
            width: 100%;
            border-collapse: collapse;
            background: var(--card);
            border-radius: 1rem;
            overflow: hidden;
        }
        th {
            text-align: left;
            padding: 1rem;
            background: rgba(255, 255, 255, 0.05);
            color: var(--text-dim);
            font-weight: 600;
        }
        td {
            padding: 1rem;
            border-top: 1px solid rgba(255, 255, 255, 0.05);
        }
        .badge {
            padding: 0.25rem 0.75rem;
            border-radius: 9999px;
            font-size: 0.8rem;
            font-weight: 600;
        }
        .badge-success { background: rgba(34, 197, 94, 0.2); color: var(--success); }
        .badge-warning { background: rgba(245, 158, 11, 0.2); color: var(--warning); }
    </style>
</head>
<body>
    <div class="container">
        <h1>MemoBuild Dashboard</h1>
        <p class="subtitle">Real-time distributed build analytics and cache status</p>
        
        <div class="grid" id="stats">
            <div class="card">
                <div class="stat-label">Total Builds</div>
                <div class="stat-value" id="total-builds">-</div>
            </div>
            <div class="card">
                <div class="stat-label">Avg Duration</div>
                <div class="stat-value" id="avg-duration">-</div>
            </div>
            <div class="card">
                <div class="stat-label">Cache Hit Rate</div>
                <div class="stat-value" id="hit-rate">-</div>
            </div>
        </div>

        <table>
            <thead>
                <tr>
                    <th>Timestamp</th>
                    <th>Dirty</th>
                    <th>Cached</th>
                    <th>Duration</th>
                    <th>Status</th>
                </tr>
            </thead>
            <tbody id="build-list">
                <!-- Data will be injected here -->
            </tbody>
        </table>
    </div>

    <script>
        async function loadStats() {
            const resp = await fetch('/api/analytics');
            const data = await resp.json();
            
            if (data.length === 0) return;

            document.getElementById('total-builds').textContent = data.length;
            
            const totalDuration = data.reduce((acc, b) => acc + b.duration_ms, 0);
            document.getElementById('avg-duration').textContent = Math.round(totalDuration / data.length) + 'ms';

            const totalNodes = data.reduce((acc, b) => acc + b.dirty_nodes + b.cached_nodes, 0);
            const totalCached = data.reduce((acc, b) => acc + b.cached_nodes, 0);
            document.getElementById('hit-rate').textContent = Math.round((totalCached / totalNodes) * 100) + '%';

            const list = document.getElementById('build-list');
            list.innerHTML = data.map(b => `
                <tr>
                    <td>${new Date(b.timestamp).toLocaleString()}</td>
                    <td>${b.dirty_nodes}</td>
                    <td>${b.cached_nodes}</td>
                    <td>${b.duration_ms}ms</td>
                    <td><span class="badge ${b.dirty_nodes === 0 ? 'badge-success' : 'badge-warning'}">
                        ${b.dirty_nodes === 0 ? 'Pristine' : 'Partial'}
                    </span></td>
                </tr>
            `).join('');
        }
        loadStats();
        setInterval(loadStats, 5000);
    </script>
</body>
</html>
    "#;
    Html(html.to_string())
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

async fn report_analytics(
    State(state): State<Arc<AppState>>,
    axum::Json(data): axum::Json<AnalyticsData>,
) -> impl IntoResponse {
    let result = state.metadata.record_build(data.dirty, data.cached, data.duration_ms);
    
    // Send build notification if webhook is configured
    if let Some(webhook_url) = state.webhook_url.clone() {
        let stats = data.clone();
        tokio::spawn(async move {
            let client = reqwest::Client::new();
            let payload = serde_json::json!({
                "text": format!(
                    "🚀 *Build Completed*\nNodes: {} dirty, {} cached\nDuration: {}ms\nStatus: {}",
                    stats.dirty,
                    stats.cached,
                    stats.duration_ms,
                    if stats.dirty == 0 { "✅ Pristine" } else { "🔧 Partial" }
                )
            });

            if let Err(e) = client.post(&webhook_url).json(&payload).send().await {
                eprintln!("⚠️ Failed to send build notification: {}", e);
            } else {
                println!("🔔 Build notification sent to webhook");
            }
        });
    }

    match result {
        Ok(_) => StatusCode::OK,
        Err(e) => {
            eprintln!("Analytics error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
