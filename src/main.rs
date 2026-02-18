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

    // Support pulling an image: memobuild pull <registry>/<repo>:<tag>
    if args.len() >= 3 && args[1] == "pull" {
        let full_name = &args[2];
        let (registry_repo, tag) = full_name.split_once(':').unwrap_or((full_name, "latest"));
        let (registry, repo) = registry_repo.split_once('/').unwrap_or(("index.docker.io", registry_repo));

        let output_dir = env::current_dir()?.join(".memobuild-cache").join("images").join(full_name.replace(':', "-").replace('/', "-"));
        let client = oci::registry::RegistryClient::new(registry, repo);
        client.pull(tag, &output_dir)?;
        return Ok(());
    }

    // Support generating CI: memobuild generate-ci --type github
    if args.len() >= 2 && args[1] == "generate-ci" {
        let yaml = r#"name: MemoBuild CI
on: [push]
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Install MemoBuild
        run: cargo install --path .
      - name: Build with Remote Cache
        run: memobuild
        env:
          MEMOBUILD_REMOTE_URL: ${{ secrets.MEMOBUILD_REMOTE_URL }}
      - name: Push Image
        run: memobuild --push
        env:
          MEMOBUILD_REGISTRY: ghcr.io
          MEMOBUILD_REPO: ${{ github.repository }}
          MEMOBUILD_TOKEN: ${{ secrets.GITHUB_TOKEN }}
"#;
        fs::create_dir_all(".github/workflows")?;
        fs::write(".github/workflows/memobuild.yml", yaml)?;
        println!("✅ GitHub Actions workflow generated at .github/workflows/memobuild.yml");
        return Ok(());
    }

    // Support generating Kubernetes manifests: memobuild generate-k8s
    if args.len() >= 2 && args[1] == "generate-k8s" {
        let yaml = r#"apiVersion: batch/v1
kind: Job
metadata:
  name: memobuild-job
spec:
  template:
    spec:
      containers:
      - name: memobuild
        image: memobuild-client:latest
        command: ["memobuild", "--push"]
        env:
        - name: MEMOBUILD_REMOTE_URL
          value: "http://memobuild-server:8080"
        - name: MEMOBUILD_REGISTRY
          value: "ghcr.io"
        - name: MEMOBUILD_REPO
          value: "your-org/your-repo"
        - name: MEMOBUILD_TOKEN
          valueFrom:
            secretKeyRef:
              name: regcred
              key: .dockerconfigjson
      restartPolicy: OnFailure
  backoffLimit: 4
"#;
        fs::create_dir_all(".k8s")?;
        fs::write(".k8s/memobuild-job.yml", yaml)?;
        println!("✅ Kubernetes Job manifest generated at .k8s/memobuild-job.yml");
        return Ok(());
    }

    // Support starting the server: memobuild --server --port 8080
    if args.iter().any(|arg| arg == "--server") {
        #[cfg(feature = "server")]
        {
            let port = args.iter()
                .position(|arg| arg == "--port")
                .and_then(|i| args.get(i + 1))
                .and_then(|p| p.parse::<u16>().ok())
                .unwrap_or(8080);
            
            let webhook_url = args.iter()
                .position(|arg| arg == "--webhook")
                .and_then(|i| args.get(i + 1))
                .cloned()
                .or_else(|| env::var("MEMOBUILD_WEBHOOK_URL").ok());

            let data_dir = env::current_dir()?.join(".memobuild-server");
            fs::create_dir_all(&data_dir)?;
            
            server::start_server(port, data_dir, webhook_url).await?;
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
    
    // 2.2 Automatic Base Image Pulling
    for instr in &instructions {
        if let docker::parser::Instruction::From(img) = instr {
            if img.contains('/') || img.contains(':') {
                println!("📦 Checking base image: {}...", img);
                let (registry_repo, tag) = img.split_once(':').unwrap_or((img, "latest"));
                let (registry, repo) = if registry_repo.contains('/') {
                    registry_repo.split_once('/').unwrap()
                } else {
                    ("index.docker.io", registry_repo)
                };

                let image_cache_dir = env::current_dir()?.join(".memobuild-cache").join("images").join(img.replace(':', "-").replace('/', "-"));
                if !image_cache_dir.exists() {
                    println!("   📥 Base image not found locally, pulling...");
                    let client = oci::registry::RegistryClient::new(registry, repo);
                    let _ = client.pull(tag, &image_cache_dir);
                }
            }
        }
    }

    println!("📊 Building DAG...");
    let mut graph = docker::dag::build_graph_from_instructions(instructions);

    println!("🔍 Detecting changes (filesystem hashing)...");
    core::detect_changes(&mut graph);

    println!("🔄 Propagating dirty flags...");
    core::propagate_dirty(&mut graph);

    let dirty = graph.nodes.iter().filter(|n| n.dirty).count();
    println!("   {} dirty  |  {} cached", dirty, graph.nodes.len() - dirty);

    // 2.5 Smart Prefetching
    if dirty > 0 {
        println!("🚀 Initiating smart prefetching for {} nodes...", dirty);
        let dirty_hashes: Vec<String> = graph.nodes.iter()
            .filter(|n| n.dirty)
            .map(|n| n.hash.clone())
            .collect();
        cache.clone().prefetch_artifacts(dirty_hashes);
    }

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
