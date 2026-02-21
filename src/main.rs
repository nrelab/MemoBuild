use memobuild::{cache, core, docker, executor, export, logging};

#[cfg(feature = "server")]
use memobuild::server;

use anyhow::Result;
use std::env;
use std::fs;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize structured logging
    let json_logs = env::var("MEMOBUILD_JSON_LOGS")
        .map(|v| v.to_lowercase() == "true")
        .unwrap_or(false);
    logging::init_logging(json_logs).ok();

    let args: Vec<String> = env::args().collect();

    // Support pulling an image: memobuild pull <registry>/<repo>:<tag>
    if args.len() >= 3 && args[1] == "pull" {
        let full_name = &args[2];
        let (registry_repo, tag) = full_name.split_once(':').unwrap_or((full_name, "latest"));
        let (registry, repo) = registry_repo
            .split_once('/')
            .unwrap_or(("index.docker.io", registry_repo));

        let output_dir = env::current_dir()?
            .join(".memobuild-cache")
            .join("images")
            .join(full_name.replace([':', '/'], "-"));
        let client = export::registry::RegistryClient::new(registry, repo);
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
            let port = args
                .iter()
                .position(|arg| arg == "--port")
                .and_then(|i| args.get(i + 1))
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080);

            let webhook_url = env::var("MEMOBUILD_WEBHOOK").ok();

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

    // Support starting the execution scheduler: memobuild --scheduler --port 9000
    if args.iter().any(|arg| arg == "--scheduler") {
        #[cfg(feature = "remote-exec")]
        {
            let port = args
                .iter()
                .position(|arg| arg == "--port")
                .and_then(|i| args.get(i + 1))
                .and_then(|p| p.parse().ok())
                .unwrap_or(9000);

            let workers_raw = env::var("MEMOBUILD_WORKERS").unwrap_or_default();
            let mut workers: Vec<Arc<dyn memobuild::remote_exec::RemoteExecutor>> = Vec::new();
            for url in workers_raw.split(',') {
                if !url.is_empty() {
                    workers.push(Arc::new(
                        memobuild::remote_exec::client::RemoteExecClient::new(url),
                    ));
                }
            }

            let strategy = match env::var("MEMOBUILD_STRATEGY").as_deref() {
                Ok("DataLocality") => {
                    memobuild::remote_exec::scheduler::SchedulingStrategy::DataLocality
                }
                Ok("Random") => memobuild::remote_exec::scheduler::SchedulingStrategy::Random,
                _ => memobuild::remote_exec::scheduler::SchedulingStrategy::RoundRobin,
            };

            let scheduler = Arc::new(memobuild::remote_exec::scheduler::Scheduler::new(
                workers, strategy,
            ));
            let server = memobuild::remote_exec::server::ExecutionServer::new(scheduler);
            server.start(port).await?;
            return Ok(());
        }
        #[cfg(not(feature = "remote-exec"))]
        {
            anyhow::bail!(
                "Remote Execution feature not enabled. Rebuild with --features remote-exec"
            );
        }
    }

    // Support starting a worker node: memobuild --worker --port 9001
    if args.iter().any(|arg| arg == "--worker") {
        #[cfg(feature = "remote-exec")]
        {
            let port = args
                .iter()
                .position(|arg| arg == "--port")
                .and_then(|i| args.get(i + 1))
                .and_then(|p| p.parse().ok())
                .unwrap_or(9001);

            let worker_id =
                env::var("MEMOBUILD_WORKER_ID").unwrap_or_else(|_| "worker-local".into());

            // Worker needs a cache and sandbox
            let cache = Arc::new(memobuild::cache::HybridCache::new(None)?);
            let sandbox: Arc<dyn memobuild::sandbox::Sandbox> =
                if args.iter().any(|arg| arg == "containerd") {
                    #[cfg(feature = "containerd")]
                    {
                        Arc::new(
                            memobuild::sandbox::containerd::ContainerdSandbox::new(
                                "unix:///run/containerd/containerd.sock",
                            )
                            .await?,
                        )
                    }
                    #[cfg(not(feature = "containerd"))]
                    {
                        anyhow::bail!("Containerd feature not enabled")
                    }
                } else {
                    Arc::new(memobuild::sandbox::local::LocalSandbox)
                };

            let worker = Arc::new(memobuild::remote_exec::worker::WorkerNode::new(
                &worker_id, cache, sandbox,
            ));
            let server = memobuild::remote_exec::worker_server::WorkerServer::new(worker);
            server.start(port).await?;
            return Ok(());
        }
        #[cfg(not(feature = "remote-exec"))]
        {
            anyhow::bail!(
                "Remote Execution feature not enabled. Rebuild with --features remote-exec"
            );
        }
    }

    println!("🚀 MemoBuild Engine Starting...");

    // 0. Collect Environment Fingerprint
    let env_fp = memobuild::env::EnvFingerprint::collect();
    println!("   🔑 Env Fingerprint: {}", &env_fp.hash()[..8]);

    // 1. Initialize Cache
    let remote_cache: Option<Arc<dyn memobuild::remote_cache::RemoteCache>> =
        if let Ok(regions_raw) = env::var("MEMOBUILD_REGIONS") {
            // Multi-region Setup: MEMOBUILD_REGIONS="https://asia.cache=asia,https://us.cache=us"
            let mut regions = Vec::new();
            for pair in regions_raw.split(',') {
                if let Some((url, name)) = pair.split_once('=') {
                    let client = Arc::new(memobuild::remote_cache::HttpRemoteCache::new(
                        url.to_string(),
                    ));
                    regions.push(Arc::new(memobuild::remote_router::RegionNode::new(
                        name, url, client,
                    )));
                }
            }

            if regions.is_empty() {
                let remote_url = env::var("MEMOBUILD_REMOTE_URL").ok();
                remote_url.map(|url| {
                    Arc::new(memobuild::remote_cache::HttpRemoteCache::new(url))
                        as Arc<dyn memobuild::remote_cache::RemoteCache>
                })
            } else {
                println!(
                    "🌐 Initializing Multi-Region Cache Router ({} regions)...",
                    regions.len()
                );
                let router = Arc::new(memobuild::remote_router::CacheRouter::new(
                    regions.clone(),
                    memobuild::remote_router::RoutingStrategy::LowestLatency,
                ));

                // Start health monitoring in background
                let regions_for_health = regions.clone();
                tokio::spawn(async move {
                    memobuild::remote_router::health::start_health_service(regions_for_health)
                        .await;
                });

                Some(
                    Arc::new(memobuild::remote_router::RouterRemoteCache { router })
                        as Arc<dyn memobuild::remote_cache::RemoteCache>,
                )
            }
        } else {
            // Single Region Setup
            let remote_url = env::var("MEMOBUILD_REMOTE_URL").ok();
            remote_url.map(|url| {
                Arc::new(memobuild::remote_cache::HttpRemoteCache::new(url))
                    as Arc<dyn memobuild::remote_cache::RemoteCache>
            })
        };

    let observer: Option<Arc<dyn memobuild::dashboard::BuildObserver>> =
        remote_cache.as_ref().map(|r| {
            let obs = memobuild::dashboard::RemoteObserver::new(Arc::clone(r));
            Arc::new(obs) as Arc<dyn memobuild::dashboard::BuildObserver>
        });

    let remote_executor: Option<Arc<dyn memobuild::remote_exec::RemoteExecutor>> =
        env::var("MEMOBUILD_REMOTE_EXEC").ok().map(|url| {
            Arc::new(memobuild::remote_exec::client::RemoteExecClient::new(&url))
                as Arc<dyn memobuild::remote_exec::RemoteExecutor>
        });

    let cache = Arc::new(cache::HybridCache::new_with_box(remote_cache)?);

    // 2. Prepare Dockerfile
    let dockerfile_path = args
        .iter()
        .position(|arg| arg == "--file" || arg == "-f")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
        .unwrap_or("Dockerfile");

    if !std::path::Path::new(dockerfile_path).exists() {
        if dockerfile_path == "Dockerfile" {
            // Create default
            fs::write(
                "Dockerfile",
                "FROM node:18\nWORKDIR /app\nCOPY package.json .\nRUN npm install\nCOPY . .\nRUN npm run build",
            )?;
        } else {
            anyhow::bail!("Dockerfile not found: {}", dockerfile_path);
        }
    }

    let dockerfile = fs::read_to_string(dockerfile_path)?;

    println!("📄 Parsing Dockerfile...");
    let instructions = docker::parser::parse_dockerfile(&dockerfile);

    // 2.2 Rich Dependency Analysis & Base Image Management
    for instr in &instructions {
        if let docker::parser::Instruction::From(img) = instr {
            println!("   🔍 Dependency: base image {}", img);
            if args.iter().any(|arg| arg == "--pull") {
                let (registry, repo, tag) = if let Some((registry_repo, tag)) = img.split_once(':')
                {
                    if let Some((reg, rep)) = registry_repo.split_once('/') {
                        (reg, rep, tag)
                    } else {
                        ("index.docker.io", registry_repo, tag)
                    }
                } else if let Some((reg, rep)) = img.split_once('/') {
                    (reg, rep, "latest")
                } else {
                    ("index.docker.io", img.as_str(), "latest")
                };

                let image_cache_dir = env::current_dir()?
                    .join(".memobuild-cache")
                    .join("images")
                    .join(img.replace([':', '/'], "-"));
                if !image_cache_dir.exists() {
                    println!("   📥 Base image not found locally, pulling...");
                    let client = export::registry::RegistryClient::new(registry, repo);
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

    println!("🔑 Recomputing deterministic hashes...");
    core::compute_composite_hashes(&mut graph, &env_fp);

    println!("📜 Propagating artifact manifests...");
    let manifests = core::propagate_manifests(&mut graph);

    // Upload all synthetic manifests to remote cache so workers can find them
    if let Some(ref _r) = cache.remote {
        for (hash, manifest) in manifests {
            let data = serde_json::to_vec(&manifest)?;
            let cache_clone = cache.clone();
            let hash_clone = hash.clone();
            tokio::spawn(async move {
                let _ = cache_clone.put_artifact(&hash_clone, &data).await;
            });
        }
    }

    let dirty = graph.nodes.iter().filter(|n| n.dirty).count();
    println!(
        "   {} dirty  |  {} cached",
        dirty,
        graph.nodes.len() - dirty
    );

    // Report DAG for visualization
    if let Some(ref r) = cache.remote {
        let _ = r.report_dag(&graph).await;
    }

    // 2.5 Smart Prefetching
    if dirty > 0 {
        println!("🚀 Initiating smart prefetching for {} nodes...", dirty);
        let dirty_hashes: Vec<String> = graph
            .nodes
            .iter()
            .filter(|n| n.dirty)
            .map(|n| n.hash.clone())
            .collect();
        cache.clone().prefetch_artifacts(dirty_hashes);
    }

    let reproducible = args.iter().any(|arg| arg == "--reproducible");
    if reproducible {
        println!("🔒 Reproducible build mode enabled");
    }

    let dry_run = args.iter().any(|arg| arg == "--dry-run");

    let build_start = std::time::Instant::now();

    let mut executor = executor::IncrementalExecutor::new(cache.clone())
        .with_reproducible(reproducible)
        .with_dry_run(dry_run);

    if let Some(obs) = observer {
        executor = executor.with_observer(obs);
    }

    if let Some(exec) = remote_executor {
        executor = executor.with_remote_executor(exec);
    }

    if let Some(sandbox_type) = args
        .iter()
        .position(|arg| arg == "--sandbox")
        .and_then(|i| args.get(i + 1))
    {
        match sandbox_type.as_str() {
            "containerd" => {
                #[cfg(feature = "containerd")]
                {
                    println!("🏗️  Using containerd sandbox runtime");
                    let sandbox = Arc::new(memobuild::sandbox::containerd::ContainerdSandbox::new(
                        "memobuild",
                        "/run/containerd/containerd.sock",
                    ));
                    executor = executor.with_sandbox(sandbox);
                }
                #[cfg(not(feature = "containerd"))]
                {
                    anyhow::bail!(
                        "Containerd feature not enabled. Rebuild with --features containerd"
                    );
                }
            }
            "local" => {
                println!("💻 Using local sandbox runtime");
            }
            _ => {
                anyhow::bail!("Unknown sandbox type: {}", sandbox_type);
            }
        }
    }

    executor.execute(&mut graph).await?;
    let duration = build_start.elapsed();

    // 3. Report Analytics
    let _ = cache
        .report_analytics(
            dirty as u32,
            (graph.nodes.len() - dirty) as u32,
            duration.as_millis() as u64,
        )
        .await;

    println!("📦 Exporting OCI Image...");
    let output_dir = export::export_image(&graph, "memobuild-demo:latest")?;

    // 4. Push to Registry (Optional)
    if args.iter().any(|arg| arg == "--push") {
        let registry_url =
            env::var("MEMOBUILD_REGISTRY").unwrap_or_else(|_| "localhost:5000".to_string());
        let repo = env::var("MEMOBUILD_REPO").unwrap_or_else(|_| "memobuild-demo".to_string());
        let token = env::var("MEMOBUILD_TOKEN").ok();

        let mut client = export::registry::RegistryClient::new(&registry_url, &repo);
        if let Some(t) = token {
            client.set_token(&t);
        }

        client.push(&output_dir)?;
    }

    println!("✅ Build and Export completed successfully");
    Ok(())
}
