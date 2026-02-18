mod core;
mod graph;
mod cache;
mod executor;
mod docker;
mod hasher;
mod oci;
mod remote_cache;
mod git;

#[cfg(feature = "server")]
mod server;

use anyhow::Result;
use std::fs;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    // Support starting the server: memobuild --server --port 8080
    if args.iter().any(|arg| arg == "--server") {
        #[cfg(feature = "server")]
        {
            let port = args.iter()
                .position(|arg| arg == "--port")
                .and_then(|i| args.get(i + 1))
                .and_then(|p| p.parse::<u16>().ok())
                .unwrap_or(8080);
            
            let data_dir = env::current_dir()?.join(".memobuild-server");
            fs::create_dir_all(&data_dir)?;
            
            server::start_server(port, data_dir).await?;
            return Ok(());
        }
        #[cfg(not(feature = "server"))]
        {
            anyhow::bail!("Server feature not enabled. Rebuild with --features server");
        }
    }

    println!("🚀 MemoBuild Engine Starting...");

    // 1. Initialize Cache
    let remote_url = env::var("MEMOBUILD_REMOTE_URL").ok();
    let remote_cache = remote_url.map(remote_cache::HttpRemoteCache::new);
    let cache = std::sync::Arc::new(cache::HybridCache::new(remote_cache)?);

    // 2. Prepare Dockerfile
    if !std::path::Path::new("Dockerfile").exists() {
        fs::write(
            "Dockerfile",
            "FROM node:18\nWORKDIR /app\nCOPY package.json .\nRUN npm install\nCOPY . .\nRUN npm run build",
        )?;
    }

    let dockerfile = fs::read_to_string("Dockerfile")?;

    println!("📄 Parsing Dockerfile...");
    let instructions = docker::parser::parse_dockerfile(&dockerfile);
    
    println!("📊 Building DAG...");
    let mut graph = docker::dag::build_graph_from_instructions(instructions);

    println!("🔍 Detecting changes (filesystem hashing)...");
    core::detect_changes(&mut graph);

    println!("🔄 Propagating dirty flags...");
    core::propagate_dirty(&mut graph);

    let dirty = graph.nodes.iter().filter(|n| n.dirty).count();
    println!("   {} dirty  |  {} cached", dirty, graph.nodes.len() - dirty);

    println!("⚡ Executing build...");
    let build_start = std::time::Instant::now();
    executor::execute_graph(&mut graph, cache.clone())?;
    let duration = build_start.elapsed();

    // 3. Report Analytics
    let _ = cache.report_analytics(dirty as u32, (graph.nodes.len() - dirty) as u32, duration.as_millis() as u64);

    println!("📦 Exporting OCI Image...");
    let output_dir = oci::export_image(&graph, "memobuild-demo:latest")?;

    // 4. Push to Registry (Optional)
    if args.iter().any(|arg| arg == "--push") {
        let registry_url = env::var("MEMOBUILD_REGISTRY").unwrap_or_else(|_| "localhost:5000".to_string());
        let repo = env::var("MEMOBUILD_REPO").unwrap_or_else(|_| "memobuild-demo".to_string());
        let token = env::var("MEMOBUILD_TOKEN").ok();

        let mut client = oci::registry::RegistryClient::new(&registry_url, &repo);
        if let Some(t) = token {
            client.set_token(&t);
        }

        client.push(&output_dir)?;
    }

    println!("✅ Build and Export completed successfully");
    Ok(())
}
